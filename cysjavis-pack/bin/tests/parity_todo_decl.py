#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""parity_todo_decl.py — 선언 블록 v1 파서의 **2언어 차등 하네스**(설계 P3-4 · K1).

Python 정본(`javis_todo_decl.py`)과 Rust 구현(`src/todo_decl.rs`)이 **같은 입력 파일들**을 읽어
**같은 정규화 결과**를 내는지 기계 비교한다. 불일치가 1건이라도 있으면 exit 1이다.

왜 이게 있어야 하나
    설계 §5-4 K1이 "2언어 파서 drift는 문서로 맞추면 반드시 어긋난다"고 못 박았고, 구현 첫날
    실제로 3곳이 갈렸다(ADR-4 C-1·C-2·C-3). 문서·리뷰는 이 결합을 못 막는다 — **같은 입력을
    양쪽에 먹여 결과를 대조하는 기계**만이 막는다. CI 잡(`ci-branch.yml` macos-rust-pack)이
    이 스크립트를 그대로 호출하며, 로컬 재현도 같은 명령이다.

계약 표면(ADR-4 C-2) — 대조하는 것은 정확히 이 7필드뿐이다
    name · verdict · diag_code · owner · scope · status · legacy
    ★사람이 읽는 한국어 진단 **문구는 대조하지 않는다.** 문구를 계약에 넣으면 문구를 다듬는
    순간 이 하네스가 결함을 오보하고, 계측기가 거짓말을 시작한다.

실행
    python3 cysjavis-pack/bin/tests/parity_todo_decl.py            # 골든 픽스처 전건
    python3 cysjavis-pack/bin/tests/parity_todo_decl.py --fuzz 512 # + 조합 생성 케이스 512종
    python3 cysjavis-pack/bin/tests/parity_todo_decl.py --list-mismatch 50

결정론
    난수를 쓰지 않는다. `--fuzz N`은 고정 축(axes)의 데카르트 곱을 고정 순서로 전개하고 고정
    보폭으로 N건을 고른다 — 같은 N이면 언제·어디서 돌려도 **같은 케이스 집합**이다. 난수 시드
    방식은 실패를 재현하려면 시드를 따로 보관해야 하고, 그 시드를 잃는 순간 증거가 사라진다.

표본 구조(★W9 교정 3)
    조합 표본은 **두 층**이다 — 넓은 축 전체(A) + 구조 축을 유효값에 고정한 층(B). 넓은 곱은
    99.9%가 무효 선언 공간이라(실측) 유효 선언의 판정 5분기가 갈리는지를 사실상 검사하지
    못했다. 그리고 커버리지 게이트는 **골든과 합산하지 않는다** — 합산하면 골든 30종만으로
    5분기·7코드가 채워져 조합 축이 100% 퇴화해도 통과한다(그 상태가 실제 결함이었다).
"""
import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))                 # …/cysjavis-pack/bin/tests
BIN = os.path.dirname(SELF)                                       # …/cysjavis-pack/bin
REPO = os.path.dirname(os.path.dirname(BIN))                      # 저장소 루트
FIXTURES = os.path.join(SELF, "fixtures", "todo-decl")
RUST_SRC = os.path.join(REPO, "src", "todo_decl.rs")
DUMPER_SRC = os.path.join(REPO, "scripts", "todo_decl_dump.rs")

sys.path.insert(0, BIN)
import javis_todo_decl as T                                       # noqa: E402

NONE = "-"                        # 값 부재 표기(덤퍼와 동일 — 빈 문자열은 오정렬을 숨긴다)
FIELDS = ("verdict", "diag_code", "owner", "scope", "status", "legacy")
VERDICTS = ("counted", "retired", "foreign-scope", "orphan-scope", "unclaimed")
# 이 건수 이상 조합을 **요청**했다면 5분기·7코드가 조합 표본 **단독으로** 전부 나와야 한다
# (코퍼스 퇴화 게이트 · W9 교정 3 — 골든과 합산하지 않는다).
COVERAGE_MIN_CASES = 256
# 조합 표본 중 `unclaimed`가 아닌(=유효 선언으로 파싱된) 케이스의 최소 비율(%).
# 유효 선언이 바닥이면 "깨진 선언을 양쪽이 똑같이 거부한다"만 검사한 코퍼스다.
VALID_SAMPLE_MIN_PCT = 10


def die(msg):
    """hard fail. 파리티 하네스에서 조용한 skip은 게이트를 껍데기로 만든다."""
    sys.stderr.write("parity_todo_decl: %s\n" % msg)
    raise SystemExit(1)


# ── 입력 확보 ──────────────────────────────────────────────────────────────

def load_spec():
    """골든 픽스처 대장을 읽는다. 디렉터리·대장·케이스 부재는 전부 **hard fail**이다."""
    if not os.path.isdir(FIXTURES):
        die("골든 픽스처 디렉터리가 없다: %s — 계약 SOT 부재는 skip이 아니라 실패다" % FIXTURES)
    spec_path = os.path.join(FIXTURES, "expected.json")
    if not os.path.isfile(spec_path):
        die("계약 대장이 없다: %s" % spec_path)
    with open(spec_path, encoding="utf-8") as f:
        spec = json.load(f)
    cases = spec.get("cases") or {}
    on_disk = sorted(f for f in os.listdir(FIXTURES) if f.endswith(".md"))
    if not on_disk:
        die("픽스처 디렉터리가 비어 있다: %s" % FIXTURES)
    if sorted(cases) != on_disk:
        die("픽스처 파일과 대장 항목이 어긋난다(누락/고아): 대장=%d 디스크=%d"
            % (len(cases), len(on_disk)))
    if spec.get("head_bytes") != T.HEAD_BYTES:
        die("파싱 예산(G3)이 대장과 구현에서 다르다: %r vs %r"
            % (spec.get("head_bytes"), T.HEAD_BYTES))
    return spec


# 예시 선언을 감싸는 머리말 장식(★W9 교정 1 축 — G12 마스킹).
# 선언 문법을 **설명하는** todo가 자기 자신을 은퇴시킨 결함이 정확히 이 축에서 났다.
# 축이 없으면 계측기가 그 방향에 눈이 먼다 — 실제로 24종 골든에 이 축이 0건이었다.
_EX = "<!-- javis:todo v1 owner=w scope=pack status=retired -->"
DECOR = (
    "",                                       # 장식 없음
    "```\n%s\n```\n" % _EX,                   # 펜스(백틱) 안 예시 선언
    "~~~\n%s\n~~~\n" % _EX,                   # 펜스(틸드) 안 예시 선언
    "> %s\n" % _EX,                           # 인용문 안 예시 선언
    "    %s\n" % _EX,                         # 들여쓴 코드블록 안 예시 선언
    "```\n<!-- ★ STALE 무효화 -->\n",         # 펜스 미닫힘(레거시 마커 설명 문서)
)

# 구분자 주입(★W9 교정 2 축 — G11 개행·공백 수렴). `(주입 위치, 문자)`.
# Python `splitlines()`/`\s`가 개행·공백으로 보지만 계약은 보지 않는 문자들 + 반대 방향.
SEP_INJECT = (
    ("", ""),                                 # 주입 없음
    ("open", "\x0b"),                         # `<!--` 와 마커 사이(VT)
    ("keys", "\x0c"),                         # 토큰 사이(FF)
    ("keys", "\x1c"),                         # 토큰 사이(FS — Rust White_Space 아님)
    ("keys", "\xa0"),                         # 토큰 사이(NBSP — Python `\s`·Rust 둘 다 공백)
    ("keys", "\u2028"),                       # 토큰 사이(LINE SEPARATOR)
    ("tail", "\x1f"),                         # 줄 꼬리(US)
    ("tail", "\u2029"),                       # 줄 꼬리(PARAGRAPH SEPARATOR)
    ("head", "\x85"),                         # 줄 선두(NEL)
)

# ★바이트 리터럴 축(W14 S15) — 코퍼스를 **bytes**로 만드는 유일한 축이다.
#
# 왜 필요한가: 종전 조합 코퍼스는 전부 `str`을 만들어 `encode("utf-8")` 한 벌로 기록했다. 즉
# **디코드 불가능한 바이트를 구조적으로 표현할 수 없었다.** 그런데 파싱 예산(G3)은 원시 바이트
# 기준이고 lossy 디코드는 잘못된 바이트 1개를 U+FFFD **3바이트**로 팽창시킨다 — 예산을 두 번
# 적용하는 구현(종전 Rust)은 정확히 이 팽창 구간에서만 Python과 갈렸다. 축이 없으니 `--fuzz
# 20000` 초록이 그 방향의 근거가 되지 못했다(계측기가 눈이 먼 상태). 여기서 축을 세운다.
#
# 각 값은 **선언 앞에 놓일 원시 바이트 패딩**이다(자기 줄로 끝난다 — 머리말 산문이므로 G12
# 마스킹 대상이 아니다). 길이는 예산 경계 3구간을 고르게 친다:
#   · 400 B  → 원시로는 예산 안 · 디코드하면 1200 B(예산 밖) = **이원화 구현이 갈리는 지점**
#   · 900 B  → 원시로도 선언이 경계를 가로지른다(양쪽이 같은 지점에서 잘려야 한다)
#   · 1100 B → 원시로 이미 예산 밖(양쪽 모두 선언을 못 본다 — 반대 방향 핀)
RAW_PAD = (
    b"",                                      # 패딩 없음(기존 코퍼스와 동형)
    b"\xff" * 400 + b"\n",                    # 비UTF-8 · 디코드 시 1200 B로 팽창
    b"\xed\xa0\x80" * 300 + b"\n",            # 고립 서로게이트(WTF-8) 900 B
    b"\xff" * 1100 + b"\n",                   # 예산 밖 비UTF-8
    "가".encode("utf-8") * 325 + b"\n",       # 유효 UTF-8 975 B — 선언이 경계를 가로지른다
)

# 조합 축(fuzz) — 각 축은 실제로 관측되거나 설계가 명시한 오작성 형태에서 왔다.
# 순서를 바꾸면 케이스 집합이 바뀐다(결정론의 대가) — 축을 늘릴 때는 **뒤에 덧붙여라**.
AXES = (
    # 머리말 접두 — BOM(G8)·제목 선행(G1' 완화 근거)·들여쓰기
    ("", "﻿", "# WORKER TODO\n\n", "\t"),
    # 주석 개시 형태(공백 유무 — 후보 판정 경계)
    ("<!--", "<!-- ", "<!--\t"),
    # 마커(레거시 은퇴 토큰 포함 — ADR-4 C-1의 그 지점)
    ("javis:todo", "javis:todo-retired", "Javis:todo", "javis:todo-x"),
    # 버전 토큰(G6)
    (" v1", " v2", " vx", ""),
    # 키셋(G4·G5·G6)
    (
        " owner=w scope=pack status=active",
        " owner=w scope=pack-dept-dept-1 status=active",
        " owner=w scope=pack-dept-dept-9 status=active",
        " owner=w scope=pack status=retired",
        " owner=w status=active",                       # 필수 키 누락
        ' owner="w" scope=pack status=active',          # 따옴표
        " Owner=w scope=pack status=active",            # 대문자 키
        " owner=w scope=pack status=done",              # status 값 위반
        " owner=w scope=pack status=active lane=a b",   # 값 내 공백
        " owner=w scope=pack.dept_1:a-b status=active",  # 값 문자 클래스 경계
    ),
    # 종결 형태(후행 텍스트·공백·부재)
    (" -->", "-->", " --> <!-- 메모 -->", " --> x", ""),
    # 선언 배치(★A2 자해 회귀 — 체크박스 뒤는 본문이다)
    ("header", "body", "header-dup"),
    # 본문 꼬리
    ("- [ ] a\n", "- [x] a\n- [ ] b\n", "본문만 있고 체크박스는 없다\n"),
    # 줄바꿈
    ("\n", "\r\n"),
    # 머리말 장식(G12 — 펜스·인용·들여쓰기·미닫힘)
    DECOR,
    # 구분자 주입(G11 — 유니코드 개행 후보·제어문자)
    SEP_INJECT,
    # ★바이트 리터럴 패딩(G3 예산 · W14 S15) — 축은 **뒤에 덧붙인다**(결정론 규약)
    RAW_PAD,
)

# ★유효 선언 편중 층(stratum B) — 구조 축을 **유효값에 고정**한 두 번째 조합 공간이다.
#
# 왜 필요한가: 넓은 축의 데카르트 곱은 압도적으로 무효 선언 공간이다(마커 4 × 버전 4 ×
# 종결 5 × 주입 9 만으로도 유효 조합은 1/180). 실측으로 `--fuzz 20000`에서 counted:23 /
# unclaimed:16019 — 99.9%가 무효였다. 그 상태의 코퍼스는 "깨진 선언을 양쪽이 똑같이 거부하는가"
# 만 검사하고, **유효 선언의 판정 5분기가 갈리는지**는 사실상 검사하지 않는다.
# 그래서 표본의 절반을 유효 선언 쪽에 배분한다. 결정론은 그대로다(난수 없음).
VALID_AXES = (
    AXES[0],                # 접두
    AXES[1],                # 주석 개시
    ("javis:todo",),        # 마커 고정
    (" v1",),               # 버전 고정
    AXES[4],                # 키셋(판정 5분기·진단 코드가 여기서 갈린다)
    (" -->", "-->"),        # 종결 고정
    ("header",),            # 머리말 배치 고정
    AXES[7],                # 본문 꼬리
    AXES[8],                # 줄바꿈
    DECOR,                  # 장식은 유효 선언과 **함께** 있어야 마스킹이 검사된다
    (("", ""),),            # 구분자 주입 없음
    # 유효 선언 층에서도 예산 축을 돈다 — 유효 선언이 팽창 구간 뒤에 놓이는 조합이
    # S15의 정확한 재현 형태이고, 그 조합은 이 층에서만 밀도 있게 나온다.
    RAW_PAD,
)


def fuzz_case_text(combo):
    """축 조합 하나를 실제 파일 **바이트**로 전개한다(결정론 — 입력만으로 출력이 정해진다).

    ★반환이 `str`이 아니라 `bytes`인 이유(W14 S15): 코퍼스가 `encode("utf-8")` 한 벌로만
    기록되면 **디코드 불가능한 바이트를 구조적으로 표현할 수 없다**. 예산(G3)은 원시 바이트
    기준인데 코퍼스가 그 축을 못 만들면 파리티 CI는 예산 이원화 결함에 눈이 먼다.
    """
    (prefix, opener, marker, version, keys, closer,
     placement, body, eol, decor, inject, raw_pad) = combo
    where, sep = inject
    # 기존 형태 보존: opener가 `<!--`면 마커가 바로 붙고, 아니면 공백 하나가 낀다.
    gap = "" if opener == "<!--" else " "
    if where == "keys" and keys.startswith(" "):
        # 선두 공백(버전 구분자)은 남기고 **토큰 사이** 첫 공백만 구분자로 바꾼다.
        keys = " " + keys[1:].replace(" ", sep, 1)
    decl = "%s%s%s%s%s%s%s" % (opener, sep if where == "open" else "", gap,
                               marker, version, keys, closer)
    if where == "tail":
        decl += sep
    elif where == "head":
        decl = sep + decl
    if placement == "header":
        text = decor + decl + "\n" + body
    elif placement == "header-dup":
        text = decor + decl + "\n" + decl + "\n" + body
    else:                                        # body — 첫 체크박스 뒤
        text = decor + body + decl + "\n"
    # 원시 패딩은 **접두(BOM 포함) 뒤**에 놓는다 — 앞에 놓으면 BOM이 파일 선두를 잃어 G8 축이
    # 통째로 죽는다. 접두는 str, 패딩은 bytes라 여기서 한 번만 인코딩해 이어 붙인다.
    return (prefix.replace("\n", eol).encode("utf-8")
            + raw_pad
            + text.replace("\n", eol).encode("utf-8"))


def combo_at(index, axes):
    """조합 공간의 index번째 조합(혼합 진법 전개). 전건 열거 없이 임의 지점을 집을 수 있다."""
    picked = []
    for axis in reversed(axes):
        index, r = divmod(index, len(axis))
        picked.append(axis[r])
    return tuple(reversed(picked))


def fuzz_step(total):
    """조합 공간과 서로소인 보폭. 표본이 축 주기와 공명해 한쪽으로 쏠리는 것을 막는다.

    단순 `total // n` 보폭은 축 길이의 배수와 맞물려 특정 축 값만 뽑히는 편향을 만든다
    (실측: 같은 1024건에서 counted 표본이 한 자릿수까지 줄었다). 서로소 보폭이면 표본이
    공간을 고르게 훑으면서도 **여전히 결정론**이다.
    """
    for cand in (65537, 40961, 32771, 12289, 7919, 1):
        if math.gcd(cand, total) == 1:
            return cand
    return 1


def build_stratum(n, axes, outdir, tag):
    """한 조합 공간에서 n건을 고정 규칙으로 고른다(난수 없음 — 같은 N이면 같은 집합)."""
    total = 1
    for axis in axes:
        total *= len(axis)
    n = min(n, total)
    step = fuzz_step(total)
    cases = []
    for i in range(n):
        idx = (i * step) % total
        name = "fuzz-%s-%07d.md" % (tag, idx)
        path = os.path.join(outdir, name)
        with open(path, "wb") as f:
            f.write(fuzz_case_text(combo_at(idx, axes)))   # 이미 bytes(바이트 리터럴 축)
        cases.append((name, path))
    return cases, total


def build_fuzz_cases(n, outdir):
    """조합 표본을 **두 층**으로 나눠 만든다 — 넓이(A) + 유효 선언 편중(B).

    반환 `(cases, total)` — total은 두 조합 공간 크기의 합이다.
    """
    n_b = n // 2
    a_cases, a_total = build_stratum(n - n_b, AXES, outdir, "a")
    b_cases, b_total = build_stratum(n_b, VALID_AXES, outdir, "b")
    return a_cases + b_cases, a_total + b_total


# ── 양측 판정 ──────────────────────────────────────────────────────────────

def python_records(cases, my_scope, existing):
    """Python 정본으로 정규화 레코드를 만든다(덤퍼와 **같은 필드·같은 표기**)."""
    def scope_exists(scope):
        return scope in existing

    out = {}
    for name, path in cases:
        decl, diag = T.parse(T.read_head(path))
        verdict = T.classify(decl, my_scope, scope_exists)
        out[name] = (
            verdict,
            diag.code if diag is not None else NONE,
            decl.get("owner", NONE) if decl else NONE,
            decl.get("scope", NONE) if decl else NONE,
            decl.get("status", NONE) if decl else NONE,
            "true" if (decl or {}).get("_legacy") else "false",
        )
    return out


def find_rustc():
    """rustc 위치. 없으면 hard fail — '컴파일러가 없어서 통과'는 파리티가 아니다."""
    cand = os.environ.get("CYS_RUSTC") or shutil.which("rustc") \
        or os.path.expanduser("~/.cargo/bin/rustc")
    if not (cand and os.path.exists(cand) if os.path.sep in str(cand) else shutil.which(cand)):
        die("rustc 를 찾을 수 없다(PATH · ~/.cargo/bin · CYS_RUSTC). "
            "Rust 측을 못 돌리면 파리티는 성립하지 않는다")
    return cand


def build_dumper(workdir):
    """Rust 덤퍼를 rustc 단독 컴파일한다(크레이트 의존 0 불변식의 기계 증명 — 파일 주석 참조)."""
    for p in (RUST_SRC, DUMPER_SRC):
        if not os.path.isfile(p):
            die("Rust 소스가 없다: %s" % p)
    binpath = os.path.join(workdir, "todo_decl_dump")
    cmd = [find_rustc(), "--edition", "2021", "-C", "debuginfo=0",
           DUMPER_SRC, "-o", binpath]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        die("Rust 덤퍼 컴파일 실패:\n%s%s" % (proc.stdout, proc.stderr))
    if proc.stderr.strip():
        sys.stderr.write(proc.stderr)
    return binpath


def rust_records(binpath, cases, my_scope, existing):
    """덤퍼를 돌려 TSV를 레코드로 되읽는다."""
    stdin = "".join("%s\t%s\n" % (n, p) for n, p in cases)
    proc = subprocess.run([binpath, "--my-scope", my_scope,
                           "--scopes", ",".join(sorted(existing))],
                          input=stdin, capture_output=True, text=True)
    if proc.returncode != 0:
        die("Rust 덤퍼 실행 실패(exit=%d):\n%s" % (proc.returncode, proc.stderr))
    out = {}
    for line in proc.stdout.splitlines():
        if not line:
            continue
        parts = line.split("\t")
        if len(parts) != 7:
            die("덤퍼 출력 형식 위반: %r" % line)
        out[parts[0]] = tuple(parts[1:])
    if len(out) != len(cases):
        die("덤퍼가 %d건 중 %d건만 냈다(입력 유실)" % (len(cases), len(out)))
    return out


def expected_records(spec):
    """대장(expected.json)을 같은 정규화 레코드로 변환 — 양측을 **SOT에도** 대조하기 위해서다.

    두 언어가 똑같이 틀리면 상호 대조만으로는 통과한다. 픽스처가 계약의 SOT(ADR-2)이므로
    골든 케이스에 한해 SOT 대조를 함께 건다.
    """
    out = {}
    for name, exp in spec["cases"].items():
        decl = exp.get("decl") or {}
        out[name] = (
            exp["classify"],
            exp["diag_code"] or NONE,
            decl.get("owner", NONE),
            decl.get("scope", NONE),
            decl.get("status", NONE),
            "true" if decl.get("_legacy") else "false",
        )
    return out


def diff(left, right, label_l, label_r, cases, limit):
    """레코드 두 벌을 비교해 불일치 목록을 낸다(케이스 순서 유지 — 재현 가능한 보고)."""
    bad = []
    for name, _ in cases:
        a, b = left.get(name), right.get(name)
        if a != b:
            fields = [(f, x, y) for f, x, y in zip(FIELDS, a or (), b or ()) if x != y]
            bad.append((name, fields))
    for name, fields in bad[:limit]:
        sys.stderr.write("불일치 [%s]\n" % name)
        for f, x, y in fields:
            sys.stderr.write("    %-10s %s=%-14s %s=%s\n" % (f, label_l, x, label_r, y))
    if len(bad) > limit:
        sys.stderr.write("… 외 %d건(--list-mismatch 로 확대)\n" % (len(bad) - limit))
    return bad


def tally(records, cases, field_idx):
    """레코드의 한 필드를 세어 분포를 낸다(`-`=해당 없음은 제외)."""
    counts = {}
    for name, _ in cases:
        v = records[name][field_idx]
        if v == NONE:
            continue
        counts[v] = counts.get(v, 0) + 1
    return counts


def fmt_tally(counts):
    return "{%s}" % ",".join("%s:%d" % kv for kv in sorted(counts.items()))


def dump_case(name, cases):
    """불일치 케이스의 실제 내용을 보여준다(재현의 마지막 조각)."""
    for n, p in cases:
        if n == name:
            with open(p, "rb") as f:
                return f.read(200)
    return b""


def main():
    ap = argparse.ArgumentParser(description="선언 파서 2언어 차등 하네스")
    ap.add_argument("--fuzz", type=int, default=0, metavar="N",
                    help="조합 생성 케이스 N종을 추가로 돌린다(결정론 — 난수 아님)")
    ap.add_argument("--list-mismatch", type=int, default=10, metavar="K",
                    help="보고할 불일치 최대 건수(기본 10)")
    args = ap.parse_args()

    spec = load_spec()
    my_scope = spec["my_scope"]
    existing = set(spec["existing_scopes"])

    workdir = tempfile.mkdtemp(prefix="todo-decl-parity-")
    try:
        binpath = build_dumper(workdir)

        # ① 진단 코드 집합 자체가 같은가(문구가 아니라 코드가 계약이다).
        proc = subprocess.run([binpath, "--codes"], capture_output=True, text=True)
        rust_codes = [c for c in proc.stdout.splitlines() if c]
        if rust_codes != list(T.DIAG_CODES):
            die("진단 코드 집합 불일치\n  python=%s\n  rust  =%s"
                % (list(T.DIAG_CODES), rust_codes))
        if list(spec.get("_diag_codes", [])) != list(T.DIAG_CODES):
            die("계약 파일의 코드 집합이 구현과 다르다")

        # ② 케이스 수집 — 골든 픽스처(계약) + 조합 생성(넓이).
        cases = [(n, os.path.join(FIXTURES, n)) for n in sorted(spec["cases"])]
        golden_n = len(cases)
        fuzz_total = 0
        if args.fuzz > 0:
            fdir = os.path.join(workdir, "fuzz")
            os.makedirs(fdir)
            fcases, fuzz_total = build_fuzz_cases(args.fuzz, fdir)
            cases += fcases

        py = python_records(cases, my_scope, existing)
        rs = rust_records(binpath, cases, my_scope, existing)

        bad = diff(py, rs, "py", "rs", cases, args.list_mismatch)
        # ③ 골든 케이스는 SOT(expected.json)에도 대조한다 — 양측 동시 오류 차단.
        exp = expected_records(spec)
        golden = cases[:golden_n]
        bad_sot = diff(py, exp, "impl", "sot", golden, args.list_mismatch)

        if bad:
            for name, _ in bad[:3]:
                sys.stderr.write("  케이스 %s 내용(선두 200B): %r\n"
                                 % (name, dump_case(name, cases)))
        if bad or bad_sot:
            sys.stderr.write(
                "\nFAIL 파리티 불일치 — 2언어 %d건 · SOT %d건 (케이스 %d종)\n"
                % (len(bad), len(bad_sot), len(cases)))
            return 1

        # ④ 코퍼스 퇴화 방지 — 전건이 `unclaimed/no-decl`로 몰린 코퍼스는 아무것도 검증하지
        #    않으면서 초록을 준다.
        #
        #    ★W9 교정 3: 골든과 조합을 **분리 집계**한다. 예전에는 둘을 합산해서, 골든 24종만
        #    으로 5분기·7코드가 이미 채워지므로(그게 골든의 설계다) 조합 축이 100% 퇴화해도
        #    게이트가 통과했다 — 축을 전부 무의미한 값으로 치환해도 exit=0이었다. 구조적으로
        #    발동 불가능한 게이트는 게이트가 아니라 장식이다.
        #
        #    그리고 발동 조건을 **요청 표본 수**(args.fuzz)로 건다. 산출된 케이스 수로 걸면,
        #    축이 퇴화해 조합 공간 자체가 쪼그라들었을 때(총 조합 1건 등) 게이트가 조용히
        #    비켜간다 — 정확히 막아야 할 상황에서 검사가 꺼지는 구조였다.
        golden = cases[:golden_n]
        fuzz_cases = cases[golden_n:]
        print("분포(골든 %d) verdict=%s diag_code=%s"
              % (golden_n, fmt_tally(tally(py, golden, 0)), fmt_tally(tally(py, golden, 1))))
        if fuzz_cases:
            fv = tally(py, fuzz_cases, 0)
            fc = tally(py, fuzz_cases, 1)
            print("분포(조합 %d) verdict=%s diag_code=%s"
                  % (len(fuzz_cases), fmt_tally(fv), fmt_tally(fc)))
            if args.fuzz >= COVERAGE_MIN_CASES:
                missing_v = set(VERDICTS) - set(fv)
                missing_c = set(T.DIAG_CODES) - set(fc)
                if missing_v or missing_c:
                    die("조합 축이 퇴화했다(골든 제외 · 요청 %d건) — 미도달 verdict=%s "
                        "diag_code=%s" % (args.fuzz, sorted(missing_v) or "-",
                                          sorted(missing_c) or "-"))
                # 유효 선언 표본이 바닥이면 "깨진 선언을 양쪽이 똑같이 거부한다"만 검사한 셈이다.
                valid = sum(v for k, v in fv.items() if k != "unclaimed")
                if valid * 100 < len(fuzz_cases) * VALID_SAMPLE_MIN_PCT:
                    die("조합 표본이 무효 선언 공간에만 떨어졌다 — 유효 판정 %d/%d(%.1f%%) "
                        "< 하한 %d%%" % (valid, len(fuzz_cases),
                                         100.0 * valid / len(fuzz_cases),
                                         VALID_SAMPLE_MIN_PCT))
        elif args.fuzz >= COVERAGE_MIN_CASES:
            die("조합 케이스가 %d건 요청됐는데 0건이 생성됐다(축 퇴화)" % args.fuzz)

        print("OK 파리티 일치 — 케이스 %d종(골든 %d + 조합 %d/%d) · 필드 %s"
              % (len(cases), golden_n, len(fuzz_cases), fuzz_total,
                 "+".join(FIELDS)))
        return 0
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
