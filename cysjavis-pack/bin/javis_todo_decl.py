#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_todo_decl — todo 선언 블록 v1 파서·분류기 (DESIGN_declared-state.md §4-1·§4-2).

todo 파일의 귀속·유효성을 **파일 안의 선언**으로만 판정한다(ADR-1). 파일명·경로·mtime은
판정 입력이 아니다 — 이 사고의 6개 실패가 전부 "추론으로 메운 자리"에서 났기 때문이다.
이름은 LLM이 쓰므로 단속이 구조적으로 불가능하고, mtime 임계는 6일짜리 유령을 놓쳤다(실측).

선언 형태(⚠`owner`는 **`cys list`의 role 열에 실재하는 값**이어야 한다 — 라벨공간을 벗어나면
소비자가 파일명으로 폴백하고 진단을 낸다. 설계 §4-1의 옛 예시 `owner=worker-2`가 실재하지 않는
이름이라 그대로 베낀 선언이 게이트 조인을 전패시켰다 · W13 중대 C):
    <!-- javis:todo v1 owner=worker scope=pack-dept-dept-2 lane=ghost-todo-fix status=active -->

이 모듈은 **판정만** 한다. 파일시스템을 만지지 않는다(팩 존재 확인조차 `scope_exists` 콜러블로
주입받는다) — 파서에 I/O를 넣으면 테스트가 실제 디스크 상태에 의존해 결정론이 깨지고,
골든 픽스처가 계약(ADR-2)이라는 전제 자체가 무너진다.

★2언어 파리티 주의(K1): 같은 픽스처를 Rust `src/todo_decl.rs`가 읽어 **동일 결과**를 내야 한다.
따라서 여기서 임의 관용(따옴표 허용·대소문자 무시 등)을 추가하면 즉시 drift가 난다.
관용은 금지하고, 대신 G9 진단 문자열로 "무엇이 틀렸는지"를 돌려준다.

공개 API:
    parse(head)                       -> (decl dict | None, Diag | None)
    classify(decl, my_scope, scope_exists) -> "counted"|"retired"|"foreign-scope"
                                             |"orphan-scope"|"unclaimed"
    read_head(path)                   -> str (선두 HEAD_BYTES **바이트**를 lossy 디코드)
    header_lines(text)                -> [str] (G1'+G12 머리말 영역 — 선언 후보가 될 수 있는 줄)
    is_retire_marker_line(line)       -> bool (G10' 레거시 은퇴 마커 **앵커** 판정)
    HEAD_BYTES                        -> 1024 (G3 파싱 예산)
    DIAG_CODES                        -> 언어중립 진단 코드 7종(양 언어 동일 집합)
    DECOR_CHARS                       -> 마커 앞 장식 문자 집합(Rust와 **같은 리터럴**)

★위치·구분자 계약 2건(W9 교정)이 이 파서의 최근 수정 지점이다.
    G12 머리말 마스킹  — 코드펜스·인용문·들여쓴 코드블록 안의 선언은 **문서용 예시**이지
                         이 파일의 선언이 아니다(`header_lines`). 이 규정이 없으면 선언 문법을
                         설명하는 todo가 자기 자신을 은퇴시킨다.
    G11 개행·공백 수렴 — 개행은 `\n`·`\r\n`·`\r` 셋만, 토큰 구분 공백은 스페이스·탭 둘만이다.
                         `str.splitlines()`·`\s`·`str.split()`을 쓰지 않는 이유가 이것이다.

★W13 교정 2건(reviewer1 2차 BLOCK · master 심판 2026-07-26).
    G10' 은퇴 마커 줄 앵커 — 레거시 은퇴 마커는 **그 줄이 마커 줄일 때만** 인정한다
                         (`is_retire_marker_line`). 종전에는 줄 어디에나 걸리는 부분일치라
                         `이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.` 같은
                         **평범한 머리말 산문 한 줄이 파일을 통째로 은퇴**시켰다(실측).
    G12 ⑤ 미닫힘 펜스 회수 — 머리말이 끝날 때까지 닫히지 않은 펜스는 **없었던 것으로 재판정**
                         한다. 종전에는 인라인 삼중백틱 한 줄이 그 아래 정당한 선언을 삼켰다.

★W15 교정 2건(reviewer1 3차 REVISE · master 심판 2026-07-26).
    G12 ⑤' 회수 충돌 취소 — 회수 구간과 정상 구간에 **둘 다** 후보가 있으면 회수를 취소한다
                         (`_has_candidate`). W13의 회수가 진짜 선언 + 미닫힘 펜스 안 예시를
                         가진 파일을 G7 `duplicate`로 죽이는 회귀를 낳았다(실측 재현).
                         회수는 "선언이 없을 때의 구제책"이지 "선언을 늘리는 장치"가 아니다.
    G10' 주석 안 앵커  — `<!--` 와 마커 토큰 사이에는 **장식 문자만**(`DECOR_CHARS`) 허용한다.
                         종전에는 `RETIRED`만 앵커되고 나머지 두 토큰은 주석 안 어디든
                         부분일치해서, **부정문이 파일을 은퇴시켰다**
                         (`<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->` · 실측 재현).

★파리티 계약의 경계(ADR-4 C-2): 기계 계약은 **`verdict` + `diag_code` + 선언 필드**뿐이다.
`Diag`가 나르는 한국어 문구는 사람을 위한 UX 어포던스이지 계약이 아니다 — 문구를 계약에 넣으면
문구 손질이 곧 파리티 CI 실패가 되어, 계측기가 결함을 오보하게 된다.

의존성: 파이썬 표준 라이브러리만(팩 규약 — 외부 의존 절대 금지).
"""

import re

# ── 상수 (양 언어 동일해야 하는 계약값) ─────────────────────────────────────
HEAD_BYTES = 1024                       # G3: 파싱 예산 = 선두 1 KiB (데몬 워치독 틱 보호)
SUPPORTED_VERSIONS = frozenset(["v1"])  # G6: 모르는 버전은 미선언 취급(스큐 정책 ADR-2)
REQUIRED_KEYS = ("owner", "scope", "status")   # G5: deny-by-default
VALID_STATUS = ("active", "retired")

# ── G11: 개행·공백 문자 집합 (★2언어 수렴 계약 · W9 교정 2) ─────────────────
# ★결정: **좁은 쪽(Rust)으로 수렴한다.** 개행은 `\n`·`\r\n`·`\r` 셋만, 토큰 구분 공백은
# 스페이스·탭 둘만이다.
#
# 왜 좁은 쪽인가 — 방향이 위험을 결정하기 때문이다. 넓은 쪽(Python 기본값)은
# `str.splitlines()`가 U+000B/000C/001C-001F/0085/2028/2029까지 개행으로 보고, 정규식 `\s`와
# `str.split()`이 유니코드 공백 전량을 구분자로 본다. Rust는 `\n\r`만 개행으로, `White_Space`만
# 공백으로 본다. 실측 13건이 갈렸고(W9 재현), 그중 다수가 **Python은 선언을 깨진 것으로 보고
# 배제하는데 Rust 데몬은 유효 선언으로 집계**하는 방향이었다 — HUD에 유령이 남는 조합이다.
# 넓은 쪽으로 맞추면 "제어문자 하나가 우연히 섞인 줄"이 유효 선언으로 승격되어 은퇴/집계
# 오판정 표면이 늘어난다. 좁은 쪽으로 맞추면 그런 줄은 양 언어에서 **똑같이** 문법 위반으로
# 떨어진다 — 오판정이 아니라 진단이 나간다(G9). 그래서 좁은 쪽이다.
#
# 대가(명시): U+00A0(NBSP)·U+3000 같은 "보이지 않는 공백"으로 토큰을 구분한 선언은 이제
# `bad-token`이다. 손기동 오작성은 생산자(`cys todo-path --emit-decl`)가 줄일 문제이지 파서가
# 추측으로 메울 문제가 아니라는 R10 원칙과 같은 자리다.
WS = " \t"                              # 토큰 구분 공백 = 스페이스·탭만
RE_WS = r"[ \t]"                        # 정규식에서 `\s` 대신 쓰는 클래스(유니코드 확장 차단)

# governance.rs check_todo·javis_report.py와 동일한 체크박스 규칙(단일 진실).
RE_DONE = re.compile(r"- \[[xX]\]")
RE_OPEN = re.compile(r"- \[ \]")

# G1' 후보 식별. `javis:todo` 뒤에 **공백**을 요구하는 이유: `javis:todo-retired`(레거시 은퇴
# 마커)는 v1 선언의 오작성이 아니라 **다른 토큰**이다. 공백을 요구하지 않으면 레거시 마커가
# "깨진 v1 선언"으로 잡혀 G10이 무력화된다(구→신 스큐 봉쇄 실패).
RE_DECL_CAND = re.compile(r"^<!--%s*javis:todo%s" % (RE_WS, RE_WS))
RE_DECL = re.compile(r"^<!--%s*javis:todo%s+(v\d+)%s+(.*?)%s*-->\Z"
                     % (RE_WS, RE_WS, RE_WS, RE_WS))
# G4: key=value. 값에 따옴표·공백·이스케이프 없음 = 파서가 단순해지고 2언어가 갈릴 여지가 없다.
RE_KV = re.compile(r"^([A-Za-z][A-Za-z0-9_]*)=([A-Za-z0-9._:-]+)$")
# ── G10' 레거시 은퇴 마커 (★W13 교정 1 · 줄 전체 앵커) ─────────────────────
# G10: 레거시 은퇴 마커를 은퇴 선언으로 인정한다. 없으면 기존 은퇴 파일이 전부 `unclaimed`로
# 쏟아진다(SIM-2 발견).
#
# ★종전 결함: 판정이 `re.search`(줄 어디에나 걸리는 **부분일치**)였다. G12 마스킹은 펜스·인용·
# 들여쓰기만 덮으므로 **평범한 머리말 산문**이 그대로 통과했다 — reviewer1 실측:
#     `이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.`
# 이 한 줄이 미완 2건짜리 살아있는 파일을 `retired`로 확정시켰다(파서 판정 = Rust 데몬 동조).
# ★가장 아픈 대목은 그 문구의 출처가 우리 자신이라는 것이다 — 이번 브랜치가 디렉티브에
# *"레인이 끝나면 status=retired 로 갱신하라"* 를 넣었고, 워커가 그 지침을 자기 todo 머리말에
# 적으면 자해한다.
#
# 그래서 마커는 **그 줄이 마커 줄일 때만** 인정한다(형태 2종 — `is_retire_marker_line`).
#   (i)  주석 줄   : 줄이 `<!--`로 시작하고 그 안에 마커 토큰이 있다
#   (ii) 마커 전용 줄: 줄 전체가 마커 토큰 하나와 정확히 일치한다
#
# ★(i)이 `-->`(주석 닫힘)를 요구하지 않는 이유는 실측이다. 이 조직의 유일한 실물 은퇴 마커
# (`_round/REVIEWER_GEMINI_TODO.md` · 07-11 teardown)가 **여러 줄 주석의 개시 줄**이다:
#     `<!-- ★★★★ STALE 무효화 (2026-07-11 dept-1 master 삽입 …) ★★★★`
# 같은 줄의 `-->`를 요구하면 그 파일이 즉시 `unclaimed`가 되어 07-26 유령 집계가 재발한다.
#
# 대가(명시): `★ STALE 무효화`처럼 **주석이 아닌** 줄은 꼬리 텍스트가 한 글자라도 붙으면
# 마커가 아니다(아래 `RE_STALE_INVALIDATE_FULL`이 `\Z` 앵커다). 주석 밖에서는 줄 전체가
# 마커 토큰 하나와 정확히 같아야 한다.
#
# ★W15 교정 2(reviewer1 3차 REVISE · master 심판 2026-07-26) — **주석 안에서도 앵커한다.**
# 종전에는 `RETIRED`만 `<!--` 직후로 앵커하고 `javis:todo-retired`·`stale 무효화`는 주석 안
# **어디든** 부분일치했다. 그 비대칭이 **부정문에 파일을 은퇴시킬 권한**을 줬다(실측 재현):
#     `<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->`      → retired (틀렸다)
#     `<!-- 은퇴시키려면 STALE 무효화 라고 적는다 -->`          → retired (틀렸다)
# 마커를 **설명하거나 부정하는** 문장이 마커로 읽히는 것은 W13이 산문에서 막았던 자해가
# 주석 안으로 자리만 옮긴 것이다.
#
# 두 극단은 심판에서 모두 기각됐다.
#   · 면제 제거      — 실물 은퇴 파일이 `done=142 open=2`라 면제를 떼면 그 파일이 park를 영구 차단한다.
#   · 엄격 앵커      — 실물 마커 10개가 전부 `<!-- ★★★★ STALE 무효화 (…)` 형태로 **장식이
#                      토큰 앞에 온다.** 엄격 앵커는 실물을 죽여 07-26 유령 집계를 부활시킨다.
# 채택: `<!--` 와 토큰 사이에는 **장식 문자만** 허용한다(`DECOR_CHARS`). 그 사이에 한글·라틴
# 문장 문자가 하나라도 있으면 마커로 인정하지 않는다. 결정론이고 2언어 구현이 싸다.
#
# ★"장식 문자 집합을 정의하는 순간 2언어가 갈린다"는 W13의 우려는 옳았다 — 그래서 집합을
# **하나의 리터럴 상수**로 뽑고(`DECOR_CHARS`) Rust `todo_decl::DECOR_CHARS`와 같은 문자열을
# 두며, `tests/test_todo_shared_constants.py`가 두 리터럴을 기계 대조한다. 갈릴 여지를 없애는
# 방법은 집합을 피하는 것이 아니라 집합을 **한 곳에 적고 기계로 묶는 것**이다.
#
# 집합에 무엇을 넣었나 — 실측 마커의 장식(`★`·공백)을 덮고, 마크다운에서 흔한 강조·구분
# 글리프를 더했다. 전부 **문장을 쓸 때 첫 글자로 오지 않는** 기호라 부정문·설명문이 이 관문을
# 통과할 수 없다. 한글·라틴 문자는 단 하나도 들어 있지 않다(그것이 이 집합의 유일한 계약이다).
DECOR_CHARS = " \t★☆*=#-~_+"
RE_STALE_INVALIDATE_HEAD = re.compile(r"stale%s*무효화" % RE_WS)  # 입력은 ASCII 소문자화된 줄
RE_STALE_INVALIDATE_FULL = re.compile(r"stale%s*무효화\Z" % RE_WS)
LEGACY_RETIRE_TOKEN = "javis:todo-retired"
# G4 토큰 분해 — `str.split()`(유니코드 공백 전량)을 쓰면 G11 계약이 깨진다.
RE_WS_RUN = re.compile(r"%s+" % RE_WS)

# ── G12: 머리말 마스킹 (★W9 교정 1 · 문서용 예시 선언 차단) ──────────────────
# 3개 이상의 백틱/틸드로 여는 펜스 코드블록.
FENCE_CHARS = ("`", "~")

# ── 진단 코드 (G9 · ADR-4 C-2) ──────────────────────────────────────────────
# ★이 7종이 **2언어 파리티 계약의 전부**다. Rust `src/todo_decl.rs`의 `diag::ALL`이 같은 7개
# 문자열을 같은 뜻으로 갖고, `parity_todo_decl.py`가 두 목록이 동일한지 기계 대조한다.
# 코드를 추가·개명하려면 양 언어 + 픽스처 `expected.json`을 **같은 커밋에서** 함께 고쳐야 한다.
#
#   no-decl          머리말 영역에 v1 선언도 레거시 은퇴 마커도 없다(미선언 = 정상 상태의 하나)
#   duplicate        머리말에 선언 후보가 2개 이상 — 모호성 거부(G7)
#   syntax           선언 줄의 **구조**가 깨졌다(따옴표 형태·`-->` 부재·버전 토큰 형태·후행 텍스트)
#   unknown-version  구조는 맞으나 버전 토큰을 모른다(G6 스큐 정책 — 미선언과 동일 취급)
#   bad-token        `key=value` 토큰 하나가 G4 문자 클래스를 위반한다
#   missing-keys     필수 키 3종(owner·scope·status) 중 누락이 있다(G5 deny-by-default)
#   bad-status       status 값이 active|retired 밖이다
DIAG_CODES = (
    "no-decl",
    "duplicate",
    "syntax",
    "unknown-version",
    "bad-token",
    "missing-keys",
    "bad-status",
)

# 사람이 읽는 문구 — **계약 아님**(ADR-4 C-2). 자유롭게 다듬어도 파리티 CI는 흔들리지 않는다.
DIAG_TEMPLATES = {
    "no-decl": "선언 없음",
    "duplicate": "선언 2개 이상(모호)",
    "syntax": "문법 위반(따옴표·후행 텍스트·형식 불일치)",
    "unknown-version": "미지 버전 %s",
    "bad-token": "토큰 문법 위반: %s",
    "missing-keys": "필수 키 누락: %s",
    "bad-status": "status 값 위반: %s",
}


class Diag(str):
    """진단 값 — 한국어 문구이면서 언어중립 `code`를 함께 나른다.

    `str` 서브클래스인 이유는 하나다: 기존 호출부(`decl, diag = parse(head)` · `javis_report.py:259`)
    를 **한 줄도 바꾸지 않고** 코드 노출을 추가할 수 있는 형태이기 때문이다. 별도 `parse_ex()`를
    두면 두 진입점이 생기고, 새 코드가 어느 쪽을 쓰는지에 따라 계약 인식이 갈린다.

    - `diag == "선언 없음"`·`"%s" % diag`·`json.dumps(diag)` 전부 기존과 동일하게 동작한다.
    - `diag.code`가 파리티 계약의 유일한 기계 판정 입력이다(문구는 계약 밖).
    - 한계(명시): `copy.copy`/`pickle` 왕복은 평범한 `str`가 되어 `.code`를 잃는다. 진단은
      생성 즉시 소비하는 값이라 문제되지 않으며, 직렬화가 필요하면 `.code`를 따로 실어라.
    """

    def __new__(cls, code, message):
        obj = str.__new__(cls, message)
        obj.code = code
        return obj

    def __repr__(self):
        return "Diag(%r, %r)" % (self.code, str(self))


def _diag(code, arg=None):
    """진단 값 생성(G9). 코드는 DIAG_CODES의 하나 — 2언어 공통 식별자다."""
    tpl = DIAG_TEMPLATES[code]
    return Diag(code, tpl % arg if arg is not None else tpl)


def _split_lines(text):
    """G11 개행 분해 — `\\n`·`\\r\\n`·`\\r` **셋만** 줄바꿈으로 본다.

    `str.splitlines()`를 쓰지 않는 이유가 이 함수의 존재 이유다: splitlines는 U+000B/000C/
    001C-001F/0085/2028/2029까지 갈라서 Rust `split_lines`(=`\\n\\r`만)와 어긋난다. 그 격차가
    실측 13건의 2언어 불일치를 만들었고, 위험한 방향(Python 배제 ↔ 데몬 집계)이었다.
    말미 개행이 빈 줄을 만들지 않는 성질은 splitlines와 동일하게 유지한다.
    """
    out = []
    start = i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c == "\n":
            out.append(text[start:i])
            i += 1
            start = i
        elif c == "\r":
            out.append(text[start:i])
            i += 2 if i + 1 < n and text[i + 1] == "\n" else 1
            start = i
        else:
            i += 1
    if start < n:
        out.append(text[start:])
    return out


def _ascii_lower(s):
    """**ASCII만** 소문자화한다 — Rust `str::to_ascii_lowercase`와 정확히 같은 의미다.

    `str.lower()`를 쓰지 않는 이유가 이 함수의 존재 이유다: 파이썬 `lower()`는 유니코드 전량을
    접어서 길이가 바뀔 수 있고(예: `İ` → 2문자), 켈빈기호 `K`가 `k`와 같아진다. 그러면 마커
    판정의 문자 위치가 Rust와 어긋나 2언어가 갈린다. 마커 토큰은 전부 ASCII이므로 ASCII 접기로
    충분하며, 한글(`무효화`)은 어느 쪽에서도 변형되지 않는다.
    """
    return "".join(chr(ord(c) + 32) if "A" <= c <= "Z" else c for c in s)


def is_retire_marker_line(line):
    """G10' — 이 **줄 하나**가 레거시 은퇴 마커 줄인가(앵커 판정 · W13 교정 1 + W15 교정 2).

    인정 형태는 2종뿐이다(상세 근거는 위 `DECOR_CHARS` 블록 주석).
      (i)  주석 줄: 줄이 `<!--` 로 시작하고, `<!--` 와 마커 토큰 사이에 **장식 문자만**
           (`DECOR_CHARS`) 있다. 토큰 **뒤**의 꼬리 텍스트는 허용한다 — 실물 마커가
           `<!-- ★★★★ STALE 무효화 (2026-07-11 …) ★★★★` 형태의 여러 줄 주석 개시 줄이라
           `-->`도 꼬리 부재도 요구할 수 없다(요구하면 실물 10개가 즉시 `unclaimed`가 되어
           07-26 유령 집계가 재발한다 · 실측).
      (ii) 비주석 줄: 줄 **전체**가 마커 토큰 하나와 정확히 일치한다
           (`javis:todo-retired` / `stale 무효화`). 꼬리 텍스트가 한 글자라도 붙으면 탈락한다
           (`RE_STALE_INVALIDATE_FULL`의 `\\Z` 앵커).

    ★(i)의 장식 접두 허용이 W15 교정 2의 전부다. 종전에는 `RETIRED`만 앵커되고 나머지 두
    토큰은 주석 안 **어디든** 부분일치해서, 마커를 **부정하는** 문장이 파일을 은퇴시켰다.
    이제 `<!--` 다음에 한글·라틴 문장 문자가 하나라도 오면 그 줄은 마커가 아니다.

    ── ★방어 범위 정본 (W18 교정 2 · 릴리스 노트 인용용) ────────────────────────────
    문장 문자가 **토큰 앞**에 오면 마커가 아니다. 토큰 **뒤**의 꼬리 텍스트는 마커를
    무력화하지 못한다(실물 마커가 꼬리를 요구하므로 불가피). 따라서 마커 토큰으로 시작하는
    설명·부정문은 은퇴로 읽힌다 — 마커를 **설명하려면 토큰을 문장 뒤에 두어라.**

    즉 아래 두 줄은 (i)의 계약상 **은퇴로 읽힌다**(실측 재현 · 2언어 동일):
      · `<!-- **STALE 무효화** 는 이 파일에 해당하지 않는다 -->`  (`**`가 DECOR_CHARS)
      · `<!-- javis:todo-retired 마커는 레거시 표기다 -->`
    이는 결함이 아니라 **계약의 경계**다. 꼬리 텍스트를 금지하면 실물 마커 10개가 전부
    `unclaimed`가 되어 07-26 유령 집계가 재발한다(실측). 실물 코퍼스에서 이 형태는 0건이다.
    안전한 설명 표기: `<!-- 이 파일은 STALE 무효화 대상이 아니다 -->`(토큰이 뒤 = 탈락).

    선두·말미 공백(G11 = 스페이스·탭)은 제거하고 판정한다. 들여쓰기 4칸 이상·탭은 애초에
    `header_lines`가 걸러내므로 여기서 다시 다루지 않는다.
    """
    s = line.strip(WS)
    low = _ascii_lower(s)
    if s.startswith("<!--"):
        # `<!--` 는 4바이트 ASCII — `_ascii_lower`가 길이를 바꾸지 않으므로 위치가 어긋나지 않는다.
        inner = low[4:].lstrip(DECOR_CHARS)          # ★장식 접두만 소거(문장 문자는 남는다)
        if inner.startswith("retired"):              # 옛 `<!--[ \t]*RETIRED` 계약의 일반화
            return True
        if inner.startswith(LEGACY_RETIRE_TOKEN):
            return True
        return RE_STALE_INVALIDATE_HEAD.match(inner) is not None
    if low == LEGACY_RETIRE_TOKEN:
        return True
    return RE_STALE_INVALIDATE_FULL.match(low) is not None


def _indent_of(line):
    """선두 공백/탭 런을 재서 `(들여쓴 코드블록인가, 본문 시작 오프셋)`을 낸다(G12).

    탭이 선두 런에 한 번이라도 나오면 들여쓴 코드블록으로 본다 — 탭 폭은 렌더러마다 다르고
    "탭 1개 = 4칸"을 가정하는 순간 양 언어가 갈릴 여지가 생긴다. 공백은 4개 이상이 코드블록.
    """
    n = 0
    for ch in line:
        if ch == " ":
            n += 1
        elif ch == "\t":
            return True, 0
        else:
            break
    return n >= 4, n


def _fence_run(rest):
    """줄 선두의 펜스 런 `(문자, 길이)`. 3개 미만이면 None(G12)."""
    if not rest:
        return None
    ch = rest[0]
    if ch not in FENCE_CHARS:
        return None
    n = 0
    for c in rest:
        if c != ch:
            break
        n += 1
    return (ch, n) if n >= 3 else None


def _is_candidate_line(line):
    """이 줄이 파일의 판정을 확정시킬 수 있는 **후보 줄**인가(G12 ⑤' 충돌 판정용).

    후보 = v1 선언 후보(`RE_DECL_CAND`) ∪ 레거시 은퇴 마커 줄(G10'). 둘 다 세는 이유는
    한쪽만 세면 같은 충돌이 다른 토큰으로 재발하기 때문이다 — `parse`가 선언 후보를 먼저
    보고 없을 때만 마커를 보므로, "정상 구간의 마커 vs 회수 구간의 선언"도 판정을 뒤집는다.
    """
    s = line.strip(WS)
    return RE_DECL_CAND.match(s) is not None or is_retire_marker_line(s)


def _has_candidate(lines):
    for ln in lines:
        if _is_candidate_line(ln):
            return True
    return False


def header_lines(text):
    """G1'+G12 — 선언 후보가 될 수 있는 **머리말 줄**만 골라 낸다.

    ★G1'(첫 체크박스 이전)만으로는 부족하다는 것이 W9 재현의 결론이다. 선언 문법을
    **설명하는** todo(코드펜스 안에 예시 선언을 적은 문서)가 자기 자신을 은퇴시켰다 — 그리고
    이 프로젝트에서 가장 흔한 todo가 정확히 그 종류다(선언 도입 작업 자체). 머리말 앞 영역을
    코드펜스·인용 구분 없이 전량 신뢰한 것이 결함이다.

    그래서 머리말 안에서 다음 4가지를 선언 후보에서 **제외**한다.
      ① 펜스 코드블록 내부(``` / ~~~ 3개 이상 · 여는 펜스와 같은 문자·같은 길이 이상으로 닫힘)
      ② 인용문(줄 선두 `>` · 선행 공백 3개까지 허용)
      ③ 들여쓴 코드블록(선두 공백 4개 이상 또는 탭)
      ④ 선언 줄 자체의 선두 공백은 3개 이하(=③의 여집합)
      ⑤ ★단, **미닫힘 펜스는 회수한다**(W13 교정 4 · master 심판) — 머리말이 끝날 때까지
         닫히지 않은 펜스는 **없었던 것으로 재판정**하고, 그 안에 갇혔던 줄을 ②③만 적용해
         후보로 되돌린다.

    ★엄격 G1(첫 비어있지 않은 줄)으로 되돌리지 않는 이유는 실측이다 — 기존 todo 63개 중
    56개(89%)가 `# 제목`으로 시작한다. 완화는 유지하고 구멍만 막는다(SIM-1·SIM-3).

    ★⑤가 왜 필요한가(W13 교정 4). 종전에는 펜스가 열리면 첫 체크박스까지 **무조건** 마스킹하고
    회수 규칙이 없었다. 그래서 인라인 삼중백틱 한 줄(오타·설명문 잔해)이 그 아래 **정당한
    선언을 통째로 삼켰다**. M1에서는 휴리스틱 폴백이 덮어 무해해 보이지만 M3에서 폴백이
    삭제되면 그 파일이 집계에서 사라지고, 지금도 `decl_stats.unclaimed_ratio`(M3 전환 판단
    근거)를 왜곡한다. 반대 방향도 같은 무게다 — 같은 형태로 `status=retired` 선언을 삼키면
    **은퇴시켰다고 믿은 파일이 계속 집계된다**.
    ★대가(명시): 미닫힘 펜스 안의 *예시* 선언은 이제 진짜 선언으로 읽힌다(골든 픽스처
    `30-unclosed-fence.md`가 그 계약이다). CommonMark는 미닫힘 펜스를 문서 끝까지로 보지만,
    "펜스를 닫지 않은 문서"는 오작성이고 그 오작성이 **정당한 선언을 죽이는** 쪽이 더 위험하다는
    것이 심판의 근거다. 닫힌 펜스의 예시 선언 보호(G12 ①의 본령)는 그대로다.

    회수 구간에서 펜스를 다시 해석하지는 않는다 — 펜스 런으로 시작하는 줄(여는 줄 자신과
    그 안의 다른 펜스 런)은 회수 대상에서 제외한다. 재해석은 "닫히지 않은 펜스 안의 펜스"라는
    재귀를 낳고, 그 재귀 규칙을 2언어가 똑같이 구현했다고 보장할 방법이 없다.

    ★⑤' **충돌 시 회수 취소**(W15 교정 1 · reviewer1 3차 · master 심판 2026-07-26).
    회수는 W13이 도입한 직후부터 **정당한 선언을 죽이는 회귀**를 갖고 있었다. 진짜 선언과
    미닫힘 펜스 안 예시가 같은 파일에 있으면 회수가 후보를 2개로 만들어 G7 `duplicate` →
    `unclaimed`가 됐다(회수 도입 전에는 예시가 마스킹돼 `counted`였다 · 실측 재현).
    선언 도입 작업의 todo가 정확히 그 형태다 — 자기 파일에 진짜 선언을 달고, 아래에 문법을
    설명하는 펜스를 여는 것.

    그래서 규칙을 한 줄로 고정한다: **회수 구간에 후보가 있고 정상 구간에도 후보가 있으면
    회수를 취소한다(정상 후보 우선).** 회수는 "선언이 없을 때의 구제책"이지 "선언을 늘리는
    장치"가 아니다. 여기서 '후보'는 선언 후보(`RE_DECL_CAND`)와 레거시 은퇴 마커 줄
    둘 다를 말한다 — 둘 다 파일의 판정을 확정시키는 줄이라 한쪽만 세면 같은 충돌이 다른
    토큰으로 재발한다.
    """
    out = []
    fence = None                                # 열린 펜스 (문자, 길이)
    pending = []                                # ⑤ 미닫힘으로 판명되면 회수할 줄들
    for line in _split_lines(text):
        if RE_DONE.search(line) or RE_OPEN.search(line):
            break                               # G1': 첫 체크박스 = 머리말의 끝
        indented, ind = _indent_of(line)
        rest = line[ind:]
        if fence is not None:                   # ① 펜스 안 — 닫힘만 살핀다
            run = _fence_run(rest)
            if (not indented and run is not None and run[0] == fence[0]
                    and run[1] >= fence[1] and rest[run[1]:].strip(WS) == ""):
                fence = None
                pending = []                    # 닫혔다 = 진짜 코드블록이었다 → 회수하지 않는다
            elif not indented and run is None and not rest.startswith(">"):
                pending.append(line)            # ⑤ 회수 후보로 적재(②③은 여기서도 적용)
            continue
        if indented:                            # ③④ 들여쓴 코드블록
            continue
        run = _fence_run(rest)
        if run is not None:                     # ① 펜스 개시 줄 자체도 선언이 아니다
            fence = run
            pending = []
            continue
        if rest.startswith(">"):                # ② 인용문
            continue
        out.append(line)
    if fence is not None:                       # ⑤ 미닫힘 펜스 = 없었던 것으로 재판정
        # ⑤' 단, 회수가 **기존 후보와 충돌**하면 회수를 취소한다(정상 후보 우선 · W15 교정 1).
        if not (_has_candidate(pending) and _has_candidate(out)):
            out.extend(pending)
    return out


def read_head(path, limit=HEAD_BYTES):
    """선두 limit **바이트**를 읽어 lossy 디코드한다. 실패(OSError)는 빈 문자열(fail-open).

    ★바이트 기준인 것이 핵심이다. 텍스트 모드 `f.read(1024)`는 1024 **문자**를 읽으므로
    한글이 섞이면 Rust(바이트 기준)와 절단 지점이 갈리고, 경계 근처에 선언이 있는 파일에서
    2언어 판정이 어긋난다. 경계에서 잘린 다바이트 문자는 양쪽 모두 U+FFFD가 된다
    (Python errors="replace" ≡ Rust String::from_utf8_lossy).
    """
    try:
        with open(path, "rb") as f:
            raw = f.read(limit)
    except OSError:
        return ""
    return raw.decode("utf-8", "replace")


def parse(head):
    """선언 블록을 판정한다. 반환 = (decl dict | None, Diag | None).

    유효 시 (dict, None), 미선언 시 (None, Diag("missing-keys", "필수 키 누락: scope")) 형태다.
    `Diag`는 str이므로 기존 호출부는 그대로 동작하고, `.code`가 언어중립 계약값이다.

    레거시 은퇴 마커(G10)만 있는 경우 owner/scope는 **미상**이라 센티널 `"?"`를 채운다 —
    {"owner": "?", "scope": "?", "status": "retired", "_legacy": True} (ADR-4 C-3).
    Rust `Decl`의 필드가 비-Option `String`이라 어차피 값을 채워야 하고, 키를 아예 빼면
    2언어 선언 표현이 갈린다. 센티널이 "모른다"를 명시적으로 말한다. 이 값이 scope 판정에
    새지 않는 것은 `classify`의 retired 단락으로 보장되며, 그 불변식은 테스트로 핀돼 있다.

    G1'(위치 계약)이 이 함수의 핵심 방어다: 선언은 **첫 체크박스 이전 머리말 영역**에만
    유효하다. 체크박스가 시작되면 그 뒤는 본문이므로, 본문에 선언 문자열을 적어 스스로를
    은퇴시키는 자해(A2 회귀)가 원천 차단된다. 첫 줄 강제(초안 G1)를 쓰지 않은 이유는 실측이다
    — 기존 todo 63개 중 56개(89%)가 `# 제목`으로 시작하므로 첫 줄 강제는 작성 습관과 충돌해
    채택률을 죽인다(SIM-1·SIM-3).

    ★G12(머리말 마스킹 · W9 교정 1)가 G1'의 나머지 절반이다: 머리말 **앞쪽**을 코드펜스·인용
    구분 없이 전량 신뢰하면, 선언 문법을 *설명하는* todo(펜스 안 예시 선언)가 자기 자신을
    은퇴시킨다. 실제로 그 종류가 이 프로젝트에서 가장 흔한 todo다. 상세는 `header_lines`.

    관용 파싱을 하지 않는다: 따옴표·값 내 공백·대문자 키는 전부 미선언이다(R10). 관용은
    2언어 불일치의 씨앗이고, 손기동 오작성은 생산자(`cys todo-path --emit-decl`)가 선언을
    기계 생성해 줄여야 할 문제이지 파서가 추측으로 메울 문제가 아니다.
    """
    if not head:
        return None, _diag("no-decl")
    text = head.lstrip("\ufeff")                    # G8: BOM 선행 제거 후 판정

    # G1'+G12: 머리말 영역 = 첫 체크박스 이전 **중 코드펜스·인용·들여쓰기 밖**(header_lines).
    header = header_lines(text)

    cands = [ln.strip(WS) for ln in header if RE_DECL_CAND.match(ln.strip(WS))]
    if not cands:
        for line in header:                         # G10': 레거시 은퇴 **마커 줄**도 은퇴로 인정
            if is_retire_marker_line(line):
                # ADR-4 C-3 — owner/scope 미상은 센티널 `"?"`. Rust와 **동일한 선언 표현**이다.
                return {"owner": "?", "scope": "?",
                        "status": "retired", "_legacy": True}, None
        return None, _diag("no-decl")
    if len(cands) > 1:                              # G7: 모호성 거부(결정론)
        return None, _diag("duplicate")

    m = RE_DECL.match(cands[0])
    if not m:
        return None, _diag("syntax")
    ver, body = m.group(1), m.group(2)
    # 후행 텍스트 방어. `-->` 뒤에 무언가 더 붙으면 lazy 그룹이 뒤쪽 `-->`까지 삼켜 body에
    # `-->`가 남는다. 이때 토큰 위반으로 흘려보내면 사용자에게 `토큰 문법 위반: -->`이라는
    # 쓸모없는 진단이 간다 — 진짜 원인(후행 텍스트)을 그대로 말해준다.
    if "-->" in body:
        return None, _diag("syntax")
    if ver not in SUPPORTED_VERSIONS:               # G6: 모르는 버전 = 미선언
        return None, _diag("unknown-version", ver)

    decl = {}
    # G11: `body.split()`(유니코드 공백 전량)이 아니라 스페이스·탭 런으로만 가른다.
    for tok in RE_WS_RUN.split(body):
        if not tok:
            continue
        km = RE_KV.match(tok)
        if not km:
            return None, _diag("bad-token", tok)
        decl[km.group(1)] = km.group(2)             # G6: 모르는 키는 그대로 담고 무시(전방호환)
    missing = [k for k in REQUIRED_KEYS if k not in decl]
    if missing:
        return None, _diag("missing-keys", ",".join(missing))
    if decl["status"] not in VALID_STATUS:
        return None, _diag("bad-status", decl["status"])
    return decl, None


def classify(decl, my_scope, scope_exists):
    """설계 §4-2 판정 5분기. 반환 = 소문자 케밥 문자열(파리티 비교용 직렬화 계약).

    인자:
        decl         parse()가 돌려준 dict 또는 None
        my_scope     내 팩 식별자("pack", "pack-dept-dept-2" …)
        scope_exists (scope: str) -> bool 콜러블. 디스크 조회를 **주입**받는 이유는 두 가지다
                     — ①파서를 결정론으로 유지(픽스처가 계약) ②판정 입력이 "디렉터리 존재"
                     라는 시간 비의존 사실임을 타입으로 못 박는다.

    scope ≠ 내 팩을 무조건 조용히 배제하지 않는 것이 R2 교정의 핵심이다. 부서 teardown·
    재생성·팩 개명은 이 조직에서 실제로 일어났고, 그때 살아있는 파일이 통째로 "남의 것"으로
    사라지면 07-11 사고를 거울상으로 재현한다. 실재하는 팩을 가리키면 정상(조용한 배제),
    실재하지 않으면 orphan-scope로 **시끄럽게** 보고한다(ADR-3 fail-open).
    """
    if not decl:
        return "unclaimed"
    # ★분기 순서가 불변식이다(ADR-4 C-3): 레거시 은퇴 선언의 `scope="?"` 센티널이 scope 판정에
    # 새면 존재하지 않는 팩으로 읽혀 은퇴 파일이 orphan-scope로 시끄럽게 쏟아진다. retired가
    # 먼저 단락되므로 **은퇴 판정에는 scope_exists가 아예 호출되지 않는다** — 테스트가 이를 핀한다.
    if decl.get("status") == "retired":             # 레거시 은퇴(_legacy)도 여기서 흡수된다
        return "retired"
    scope = decl.get("scope")
    if scope == my_scope:
        return "counted"
    return "foreign-scope" if scope_exists(scope) else "orphan-scope"
