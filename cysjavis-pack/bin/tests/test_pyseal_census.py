#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_pyseal_census.py — SEAL-1(PYTHONDONTWRITEBYTECODE) 강제점 census 회귀 핀 (팩 측 · 2언어 미러 · standalone).

★왜 이 파일이 존재하는가(A6 · 2026-09-03 · CEO 승인 정의 = ceo/bugfix-2026-09-03/impl/W-A/DESIGN.md §A6):
  SEAL-1(2026-08-01 실사고 — 번들 python 이 번들 안에 `__pycache__/*.pyc` 를 써서 코드서명 봉인이
  깨지고 Gatekeeper 가 "손상되었기 때문에 열 수 없습니다"로 차단 · 정본 src/lib.rs `ENV_PY_NO_BYTECODE`)
  은 3층으로 봉인돼 있고 각 층은 **지점별** 핀이 지킨다 — Rust: `python_command` 팩토리 핀 ·
  `spawn_env_pairs` 핀 · 직스폰 전수 열거 핀 `bundled_python_spawn_sites_are_enumerated_and_sealed`
  (src/lib.rs:2536-2643 · cargo test) · 훅 셸 층: H-PYSEAL-1(run_bootstrap_health.py:4074).
  그러나 **강제점 전체 집합을 한 자리에서 세는** 검체는 없었다: 강제점 하나가 조용히 사라지거나
  (예: cys-dept 헤드 export 삭제) 새 진입점(훅 · bin 셸 스크립트 · Rust 직스폰)이 봉인 없이 생겨도,
  cargo 테스트를 안 도는 팩 레인(pack-release · 팩 단독 CI)은 초록이다. 0.14.27 의 SEAL-1 3중 구현은
  **신규 구현 대상이 아니라 census 회귀 테스트로 유지 여부만 확인**한다(CLAUDE.md 정정 기록
  2026-08-29) — 이 파일이 그 census 다.

census 정의(CEO 승인 · 표는 전부 grep 실측값으로 고정 — 추정 0 · 초판 2026-09-03 · ⓐ 확장 2026-09-04):
  ⓐ 강제점 집합 — 비주석 코드의 강제/정의 **17점(패턴 18줄)** 을 파일별 **정확 개수**로 고정.
     0 = 봉인 소실(SEAL-1 재발 경로). 초과도 FAIL 이다 — 중복 강제는 무해하지만 "어느 지점이 정본인가"가
     흐려진다(늘렸으면 봉인을 확인하고 이 표에 등재하라).
     ★2026-09-04 갱신(A6 후속 · **11점 12줄 → 17점 18줄** · 추가 6줄 = 6점) — 사유: 초판 표가 강제점
     3종을 놓쳤고 셋 다 **변조로 실증**했다(각각 그 줄만 지워도 census 가 `PYSEAL-CENSUS-OK`/exit 0 =
     초록이었다). 즉 "census 가 초록이니 봉인이 전수 유지된다"는 문장이 그 사이 거짓이었다.
       ① `src/bin/cys.rs:16578` `run_scoped`(= `cys run -- <명령>` 임의 명령 스폰의 방어심도) — ⓑ 는
          같은 파일 주석(:8709·:16577)이 니들을 계속 보유해 집합에서 안 빠지므로 못 잡고, ⓒ(iii) 은
          스폰 줄이 `Command::new(&command[0])`(:16573)라 "py"/"python" 이 없어 애초에 지점으로 세지
          않으며, src/lib.rs 의 봉인기전 marker assert(:2624-2628)도 cys.rs 를 목록에 넣지 않았다 —
          2언어 어느 핀도 이 지점을 지키지 않았다.
       ② 층3 호출부 3곳(`src/bin/cys.rs:1257` · `src/bin/cysd/main.rs:1089` ·
          `src-tauri/src/main.rs:5748`) — 층3 은 함수 정의(src/lib.rs:159)만으로는 무효이고 이 세 호출이
          있어야 성립한다(src/lib.rs:150-155 "스레드가 생기기 전 각 바이너리 main 첫 줄" 계약).
          src/lib.rs:2977 핀은 함수를 **직접 호출해** 행동만 검사하므로 호출부 소멸을 보지 못한다.
       ③ 켜짐 값 상수 `src/lib.rs:97 PY_NO_BYTECODE_ON = "1"` — 빈 값이면 CPython 이 '끔'으로 읽어
          층1·층2·층3 이 **동시에** 죽는다(값 규약 근거 src/lib.rs:95-96). cargo 측 핀
          (src/lib.rs:2503 `assert!(!PY_NO_BYTECODE_ON.is_empty(), …)`)은 잡지만, 이 census 의 존재
          이유가 바로 "cargo 를 안 도는 팩 레인"이라 같은 계급의 구멍이었다.
     (표가 세는 것은 줄 번호가 아니라 **비주석 줄의 개수**다 — 주석의 행번호는 2026-09-04 grep 재실측.)
  ⓑ 참조 파일 집합 — `PYTHONDONTWRITEBYTECODE`/`ENV_PY_NO_BYTECODE` 를 언급하는 파일 **18개 목록** 고정
     (2026-09-04 W-A A2: 17→18 · 훅 런처/본체 분할 검체 등재 — 등재 사유는 목록 주석 참조)
     (확장자 13종 · target/node_modules/_worktrees/.git 제외 · 이 파일 자신 제외 · grep -rl 동형).
     신규/소멸 → FAIL: 그 파일의 봉인을 점검한 뒤 목록을 갱신하라(갱신 = "봉인을 확인했다"는 선언).
  ⓒ 신규 진입점 검출(양방향):
     (i)  hooks/**/*.sh(_lib.sh 제외) 가운데 **비주석 줄**에서 `python`/`CYS_PY` 를 쓰는 훅은 `_lib.sh`
          source 필수(+ 소비 훅 하한 29 — 술어가 깨져 0건이 되는 공허 통과 차단). 비주석 한정인 이유:
          cys-statusline.sh 는 주석 "python 등 외부 의존 없음"이 명문 계약인 의존 0 훅이다
          (run_bootstrap_health.py:727-731 의 의도적 제외와 동일 판정).
     (ii) bin/ 직속 비-python 셸 스크립트(셔뱅 sh/bash 판정 · 실행비트 무관 = Windows 체크아웃 안전)가
          비주석 줄에서 `python` 을 부르면 `export PYTHONDONTWRITEBYTECODE=1`(또는 _lib.sh 2줄형
          `PYTHONDONTWRITEBYTECODE=1` + `export PYTHONDONTWRITEBYTECODE` · 값 "1" 고정 = 빈 값은 CPython 이
          '끔'으로 읽는다) 또는 `_lib.sh` source 필수. 현 미봉인 3건은 **명시 allowlist** — 신규 미봉인 =
          FAIL · allowlist 항목의 소멸/봉인/python 미사용 = FAIL(묵은 allowlist 는 의도적으로 정리하라).
     (iii) Rust python 직스폰 지점 수 — src/lib.rs 핀의 탐지 규약(같은 줄에 `Command::new(` 가 닫히고
          인자에 "py" 또는 줄에 "python" · `//` 줄 제외 · vendor/target 제외 · ASCII 소문자화)을 **그대로
          미러**해 {cys.rs 1 · boot_supervisor 1 · cysd/main 2 · tauri/main 4} 를 2언어로 핀 — cargo 를
          안 도는 레인에서도 같은 값이 같은 순간에 깨진다.

종료 코드(run_bootstrap_health 규약과 동형): 0 = 전부 측정·통과(말미 `PYSEAL-CENSUS-OK`) ·
1 = FAIL 1건 이상 · 2 = UNMEASURED(레포 루트에 src/lib.rs 부재 = 팩 단독 설치 — **측정 불능은 통과가
아니다**). 루트 해소: `PYSEAL_CENSUS_ROOT`(비어 있지 않으면 · 변조 대조용 사본 루트) → 기본
dirname(PACK)(= git 워크트리). stdlib 전용 · 쓰기 0 · 결정론(정렬 key=str · LC_ALL 무관) · 실측 <0.1s.
변조 대조(계측 타당성): ceo/bugfix-2026-09-04/impl/W-A/evidence/checks/check-A6-mutation.sh
(초판 = ceo/bugfix-2026-09-03/… · **이 포인터는 계약이 아니라 흔적이다** — 팩 밖 절대경로라 통합
리포로 병합되면 거짓이 될 수 있으니 발견 즉시 갱신하라). 실증 내용: ⓪실 루트 PASS(양성) ·
⓪′**무변조 rsync 전체 사본 PASS**(귀속 기준선 — 부분 사본은 docs/ 결손으로 ⓑ 가 항상 FAIL 해
"무엇 때문에 FAIL 했는가"가 오염된다) · ①cys.rs run_scoped 1줄 제거 ②층3 호출부 3줄 제거
③`PY_NO_BYTECODE_ON` 값 `"1"`→`""` 치환 사본이 각각 **정확히 그 항목만** FAIL(exit 1).

    CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_pyseal_census.py
"""
import os
import re
import string
import sys

# 로케일 비의존 출력(W-A4 선례 · test_seat_latch_negation 동형): cp949 파이프 캡처에서 한글 진단
# UnicodeEncodeError 즉사 방지.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.dirname(os.path.dirname(TESTS_DIR))
ROOT = os.path.abspath(os.environ.get("PYSEAL_CENSUS_ROOT") or os.path.dirname(PACK))
# ⓑ 자기 제외는 **루트 상대경로**로 — 변조 대조 사본 루트에서도 사본 test 파일이 같은 규칙으로 빠진다.
SELF_REL = "cysjavis-pack/bin/tests/" + os.path.basename(os.path.abspath(__file__))

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _lines(text):
    """Rust `str::lines()` 동형: '\\n' 분할 + 꼬리 '\\r' 제거(CRLF 체크아웃 안전 · 유니코드 행분리자는
    쪼개지 않는다 — splitlines() 와 달리 (iii) 미러 핀이 Rust 와 같은 줄 경계를 본다)."""
    out = [l[:-1] if l.endswith("\r") else l for l in text.split("\n")]
    if out and out[-1] == "":
        out.pop()
    return out


def _read_text(path):
    with open(path, "rb") as fh:
        return fh.read().decode("utf-8", "replace")


def _code_lines(text, comment):
    """비주석 줄만 [(lineno, line)]. comment = '//'(.rs · doc 주석 `///` 포함) 또는 '#'(셸 · 셔뱅 줄 포함)."""
    return [(i, l) for i, l in enumerate(_lines(text), 1) if not l.lstrip().startswith(comment)]


def _rel(path):
    return os.path.relpath(path, ROOT).replace(os.sep, "/")


# ═══════════════════════════════════════════════════════════════════════════
# ⓐ 강제점 집합 — 17점(패턴 18줄) · 비주석 코드 · 파일별 정확 개수 (행번호는 2026-09-04 실측)
#   mode "sub"  = 비주석 줄 가운데 패턴을 **부분 문자열**로 포함하는 줄 수
#   mode "line" = 비주석 줄 가운데 패턴과 **줄 전체가 정확히 같은** 줄 수(들여쓰기·조건 분기 안으로
#                 들어가면 "무조건 헤드 export" 계약이 깨진 것이므로 FAIL 이 맞다 — H-PYSEAL-1 과 동형)
#   ★2026-09-04 A6 후속으로 6줄(=6점) 추가 — 6번째는 R2 적대검증이 변조로 찾은 층2 값 줄 — 사유·변조 실증은 상단 독스트링 ⓐ 갱신 절.
# ═══════════════════════════════════════════════════════════════════════════
ENFORCEMENT = (
    ("src/lib.rs", "//", (
        ("sub", 'pub const ENV_PY_NO_BYTECODE: &str = "PYTHONDONTWRITEBYTECODE";', 1),  # 정본 상수 = 키(:94)
        ("sub", 'pub const PY_NO_BYTECODE_ON: &str = "1";', 1),                         # ★정본 상수 = 켜짐 값(:97) —
                                                                                        #   빈 값이면 CPython 이 '끔'으로
                                                                                        #   읽어 층1·2·3 동시 무효(:95-96)
        ("sub", "cmd.env(ENV_PY_NO_BYTECODE, PY_NO_BYTECODE_ON);", 1),                  # python_command 팩토리 = 층1(:133)
        ("sub", "set_var(ENV_PY_NO_BYTECODE, PY_NO_BYTECODE_ON)", 1),                   # in-process 층3 정의(:159)
        ("sub", "ENV_PY_NO_BYTECODE.to_string(),", 1),                                  # spawn_env_pairs 키 = 층2(:1173)
        ("sub", "PY_NO_BYTECODE_ON.to_string(),", 1),                                   # ★spawn_env_pairs **값**(:1174) —
                                                                                        #   층2 는 키·값을 두 줄로 push 한다.
                                                                                        #   값을 `String::new()` 로 바꾸면
                                                                                        #   pane·훅·스케줄 잡이 빈 값을 상속해
                                                                                        #   CPython 이 '끔'으로 읽는다(층2 사망).
                                                                                        #   ③과 정확히 같은 계급의 구멍이었다
                                                                                        #   (2026-09-04 R2 적대검증 변조 실증).
    )),
    ("src/bin/cys.rs", "//", (
        ("sub", "cmd.env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON);", 1),        # ★run_scoped = `cys run -- <명령>`
                                                                                        #   임의 명령 스폰 방어심도(:16578)
        ("line", "    cys::seal_python_bytecode_in_process();", 1),                      # ★층3 호출부 — cys main 선두(:1257)
                                                                                        #   line 모드인 이유: 이 항목의 계약은
                                                                                        #   '스레드 생성 전 main 선두'(lib.rs:150-155)
                                                                                        #   라 **위치**가 본질이다. sub 면 조건
                                                                                        #   분기 안으로 옮겨도 초록이다(R2 실증).
    )),
    ("src/bin/cysd/boot_supervisor.rs", "//", (
        ("sub", ".env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON)", 1),            # run_ensure_team tokio 직봉인(:816)
    )),
    ("src/bin/cysd/main.rs", "//", (
        ("sub", ".env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON)", 1),            # office-bridge tokio 직봉인(:1653)
        ("line", "    cys::seal_python_bytecode_in_process();", 1),                      # ★층3 호출부 — cysd main 선두(:1089)
                                                                                        #   (`#[tokio::main]`→async_main 인 이유)
    )),
    ("src-tauri/src/main.rs", "//", (
        ("sub", "cmd.env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON);", 1),        # inject_runtime_path(:4092)
        ("line", "    cys::seal_python_bytecode_in_process();", 1),                      # ★층3 호출부 — GUI main 선두(:5748)
    )),
    ("cysjavis-pack/hooks/_lib.sh", "#", (
        ("line", "PYTHONDONTWRITEBYTECODE=1", 1),                                       # 프리루드 무조건 대입(:265)
        ("line", "export PYTHONDONTWRITEBYTECODE", 1),                                  # + export(:266) — H-PYSEAL-1 짝
    )),
    ("cysjavis-pack/bin/cys-dept", "#", (
        ("line", "export PYTHONDONTWRITEBYTECODE=1", 1),                                # 헤드 단일 지점 export(:42)
    )),
    ("scripts/installer-app/install-core.sh", "#", (
        ("line", "export PYTHONDONTWRITEBYTECODE=1", 1),                                # DMG 설치 코어(:67)
    )),
    ("scripts/verify-gatekeeper-user-path.sh", "#", (
        ("sub", 'PYTHONDONTWRITEBYTECODE=1 "$PYB"', 1),                                 # ⑥-B SEAL-1 완화 프로브(:334)
    )),
)


def check_enforcement():
    for rel, comment, pats in ENFORCEMENT:
        path = os.path.join(ROOT, *rel.split("/"))
        if not os.path.isfile(path):
            for _mode, pat, _want in pats:
                check("ⓐ %s :: %s" % (rel, pat), False,
                      "파일 부재 — 강제점 소실 또는 이동(이동이면 봉인을 확인하고 ENFORCEMENT 표를 갱신하라)")
            continue
        code = _code_lines(_read_text(path), comment)
        for mode, pat, want in pats:
            if mode == "line":
                hits = [i for i, l in code if l == pat]
            else:
                hits = [i for i, l in code if pat in l]
            check("ⓐ %s :: %s" % (rel, pat), len(hits) == want,
                  "비주석 %d줄(기대 %d) @%s%s" % (
                      len(hits), want, ",".join(str(i) for i in hits) or "-",
                      "" if len(hits) == want else
                      " — 0이면 봉인 소실(SEAL-1 .pyc 번들 오염 재발 경로), 초과면 중복 강제(정본 흐림) "
                      "→ 봉인을 확인한 뒤 ENFORCEMENT 표를 갱신하라"))


# ═══════════════════════════════════════════════════════════════════════════
# ⓑ 참조 파일 집합 — grep -rl 동형(확장자 13종 · 4 디렉토리 제외 · 자기 제외) · 2026-09-04 실측 18
#   (확장자 없는 cysjavis-pack/bin/cys-dept 는 정의상 이 집합 밖이고 ⓐ·ⓒ(ii) 가 핀한다.)
# ═══════════════════════════════════════════════════════════════════════════
REF_EXTS = (".rs", ".sh", ".py", ".ts", ".js", ".md", ".toml", ".yml", ".yaml", ".json", ".plist", ".bat", ".ps1")
REF_SKIP_DIRS = frozenset(("target", "node_modules", "_worktrees", ".git"))
REF_NEEDLES = (b"PYTHONDONTWRITEBYTECODE", b"ENV_PY_NO_BYTECODE")
REFERENCING_FILES = (  # 정렬 key=str(코드포인트 순 · LC_ALL=C sort 와 동일) · 중복 없음
    # ★2026-09-04 W-A 등재(IG-9 워크플로 생산자 쌍 `67e4849`/`9d452d9` 로 유입).
    #   **봉인 점검 결과(등재 = 이 선언)**: 이것은 **진짜 강제점**이다 — 주석·문서 언급이 아니라
    #   스텝 env 로 `PYTHONDONTWRITEBYTECODE: '1'` 을 실제로 건다(`:255`). 그 스텝이 Windows
    #   러너에서 python 을 호출하므로 봉인이 그 실행에 **적용된다**. 따라서 니들 보유는 정상이며
    #   제거 대상이 아니다(제거하면 그 레인만 봉인이 빠진다).
    ".github/workflows/windows-build.yml",
    "cysjavis-pack/bin/tests/run_bootstrap_health.py",
    # ★2026-09-04 W-A A2 등재 — 훅 런처/본체 분할 검체. **봉인 점검 결과(등재 = 이 선언)**:
    #   python 서브프로세스를 하나도 띄우지 않는다(스폰 대상은 전부 `sh`/`dash`/`bash` 런처다) —
    #   따라서 새 python 진입점도 강제점도 아니다. 니들을 보유하는 이유는 단 하나, 프리루드의
    #   봉인(SEAL-1)이 훅 본체까지 **상속되는지 관측**하기 때문이다(PRELUDE-1b).
    "cysjavis-pack/bin/tests/test_hook_launcher_split.py",
    "cysjavis-pack/bin/tests/test_org_audit.py",
    "cysjavis-pack/hooks/_lib.sh",
    "docs/RELEASE.md",
    "docs/plans/v4-repair-spec.md",
    "scripts/deploy_gate.py",
    "scripts/installer-app/install-core.sh",
    "scripts/precompile-bundled-python.sh",
    "scripts/verify-gatekeeper-user-path.sh",
    "src-tauri/src/main.rs",
    "src/app_bundle.rs",
    "src/bin/cys.rs",
    "src/bin/cysd/boot_supervisor.rs",
    "src/bin/cysd/main.rs",
    "src/bin/cysd/schedule.rs",
    "src/bin/cysd/state.rs",
    "src/lib.rs",
)


def check_referencing():
    want = list(REFERENCING_FILES)
    check("ⓑ 핀 목록 정렬·중복 없음(%d)" % len(want), want == sorted(set(want), key=str),
          "REFERENCING_FILES 는 key=str 정렬·유일해야 diff 가 결정론이다")
    found, unreadable = [], []
    for dp, dns, fns in os.walk(ROOT):
        dns[:] = sorted((d for d in dns if d not in REF_SKIP_DIRS), key=str)
        for fn in sorted(fns, key=str):
            if not fn.endswith(REF_EXTS):
                continue
            p = os.path.join(dp, fn)
            if os.path.islink(p):  # grep -r 동형: 명령행 인자가 아닌 심링크는 따라가지 않는다
                continue
            rel = _rel(p)
            if rel == SELF_REL:
                continue
            try:
                with open(p, "rb") as fh:
                    blob = fh.read()
            except OSError as e:
                unreadable.append("%s(%s)" % (rel, e.__class__.__name__))
                continue
            if any(n in blob for n in REF_NEEDLES):
                found.append(rel)
    found = sorted(found, key=str)
    check("ⓑ 참조 파일 전수 판독", not unreadable, "판독 실패 %s — 측정 불능은 통과가 아니다" % unreadable)
    added = sorted(set(found) - set(want), key=str)
    removed = sorted(set(want) - set(found), key=str)
    detail = "실측 %d" % len(found)
    if added:
        detail += " · 신규 %s → 그 파일이 새 python 진입점/강제점인지 봉인을 점검한 뒤 REFERENCING_FILES 에 등재하라" % added
    if removed:
        detail += " · 소멸 %s → 강제점/참조 소실인지 확인한 뒤 목록에서 내려라" % removed
    check("ⓑ 참조 파일 집합 %d개 고정" % len(want), not added and not removed, detail)


# ═══════════════════════════════════════════════════════════════════════════
# ⓒ(i) hooks/**/*.sh — 비주석 python/CYS_PY 소비 훅은 _lib.sh source 필수 (+ 소비 훅 하한)
# ═══════════════════════════════════════════════════════════════════════════
# `. "$(dirname "$0")/_lib.sh" …` · `. "$(dirname "$0")/../_lib.sh" …` · 2단 폴백 `|| . "${CYS_PACK_DIR…}/hooks/_lib.sh"`
LIB_SOURCE_RE = re.compile(r'^\s*(?:\|\||&&)?\s*(?:\.|source)\s+.*_lib\.sh')
HOOK_CONSUMER_FLOOR = 29  # 2026-09-03 실측 29(의존 0 훅 2 = cys-hook.sh·cys-statusline.sh 제외) — 축소 회귀·술어 붕괴 차단


def check_hooks():
    hooks_dir = os.path.join(ROOT, "cysjavis-pack", "hooks")
    if not os.path.isdir(hooks_dir):
        check("ⓒ(i) hooks 디렉토리", False, "%s 부재 — 측정 불능은 통과가 아니다" % hooks_dir)
        return
    consumers, nosrc = [], []
    for dp, dns, fns in os.walk(hooks_dir):
        dns[:] = sorted(dns, key=str)
        for fn in sorted(fns, key=str):
            if not fn.endswith(".sh") or fn == "_lib.sh":
                continue
            p = os.path.join(dp, fn)
            rel = os.path.relpath(p, hooks_dir).replace(os.sep, "/")
            code = [l for _i, l in _code_lines(_read_text(p), "#")]
            if not any(("python" in l or "CYS_PY" in l) for l in code):
                continue
            consumers.append(rel)
            if not any(LIB_SOURCE_RE.match(l) for l in code):
                nosrc.append(rel)
    check("ⓒ(i) python/CYS_PY 소비 훅 전수 _lib.sh source", not nosrc,
          "미적용 %s — 프리루드 없이 $CYS_PY/번들 python 을 부르면 SEAL-1 훅 층 밖이다(H-PYSEAL-1 참조): "
          "훅 선두에 2단 source 프리루드를 넣어라" % nosrc if nosrc else "소비 훅 %d개 전부 source" % len(consumers))
    check("ⓒ(i) 소비 훅 하한 %d" % HOOK_CONSUMER_FLOOR, len(consumers) >= HOOK_CONSUMER_FLOOR,
          "실측 %d%s" % (len(consumers),
                        "" if len(consumers) >= HOOK_CONSUMER_FLOOR else
                        " — 훅 삭제면 하한을 내리고, 아니면 소비 술어(python/CYS_PY · 비주석)가 깨진 것이다"))


# ═══════════════════════════════════════════════════════════════════════════
# ⓒ(ii) bin/ 직속 비-python 셸 스크립트 — python 호출 시 봉인 필수 · 미봉인 allowlist 양방향 고정
# ═══════════════════════════════════════════════════════════════════════════
SHELL_SHEBANG_RE = re.compile(r'^#!\s*(?:\S*/)?(?:env\s+(?:-S\s+)?)?(?:sh|bash|dash|ksh|zsh)(?:\s|$)')
EXPORT_ONE_RE = re.compile(r'(?:^|[;\s])export\s+PYTHONDONTWRITEBYTECODE=1(?:[;\s]|$)')
ASSIGN_ONE_RE = re.compile(r'(?:^|[;\s])PYTHONDONTWRITEBYTECODE=1(?:[;\s]|$)')
EXPORT_BARE_RE = re.compile(r'(?:^|[;\s])export\s+PYTHONDONTWRITEBYTECODE(?:[;\s]|$)')
# 2026-09-03 실측 미봉인 3건 — 개발용 e2e/게이트 셸(호스트 python3 · 번들 밖 경로). 신규 항목 추가는
# "왜 봉인이 불필요한가"를 이 주석에 적는 결정이지, FAIL 을 끄는 스위치가 아니다.
BIN_UNSEALED_ALLOWLIST = ("javis_org_e2e.sh", "javis_purge_e2e.sh", "rsi-gate.sh")


def _shell_sealed(code):
    return (any(EXPORT_ONE_RE.search(l) for l in code)
            or (any(ASSIGN_ONE_RE.search(l) for l in code) and any(EXPORT_BARE_RE.search(l) for l in code))
            or any(LIB_SOURCE_RE.match(l) for l in code))


def check_bin():
    bin_dir = os.path.join(ROOT, "cysjavis-pack", "bin")
    if not os.path.isdir(bin_dir):
        check("ⓒ(ii) bin 디렉토리", False, "%s 부재 — 측정 불능은 통과가 아니다" % bin_dir)
        return
    info = {}  # name -> (python 호출 여부, 봉인 여부) · 셸 셔뱅 파일만(.py/.json 등은 밖)
    for name in sorted(os.listdir(bin_dir), key=str):
        p = os.path.join(bin_dir, name)
        if not os.path.isfile(p):
            continue
        text = _read_text(p)
        first = _lines(text)[0] if text else ""
        if not SHELL_SHEBANG_RE.match(first):
            continue
        code = [l for _i, l in _code_lines(text, "#")]
        info[name] = (any("python" in l for l in code), _shell_sealed(code))
    new_unsealed = sorted((n for n, (py, sealed) in info.items()
                           if py and not sealed and n not in BIN_UNSEALED_ALLOWLIST), key=str)
    check("ⓒ(ii) bin/ 셸 스크립트 신규 미봉인 0", not new_unsealed,
          ("미봉인 %s — 헤드에 `export PYTHONDONTWRITEBYTECODE=1` 또는 _lib.sh source 를 넣어 봉인하라"
           "(번들 python 이 .pyc 를 쓰면 코드서명 파손)" % new_unsealed) if new_unsealed
          else "셸 스크립트 %d개 검사(python 호출 %d · 봉인 %d · allowlist %d)" % (
              len(info), sum(1 for py, _s in info.values() if py),
              sum(1 for py, s in info.values() if py and s), len(BIN_UNSEALED_ALLOWLIST)))
    stale = []
    for n in BIN_UNSEALED_ALLOWLIST:
        if n not in info:
            stale.append("%s(소멸 또는 셸 셔뱅 아님)" % n)
        elif not info[n][0]:
            stale.append("%s(python 미사용)" % n)
        elif info[n][1]:
            stale.append("%s(봉인됨)" % n)
    check("ⓒ(ii) 미봉인 allowlist %d건 현행" % len(BIN_UNSEALED_ALLOWLIST), not stale,
          ("묵은 항목 %s — 의도적으로 BIN_UNSEALED_ALLOWLIST 에서 내려라(허수 allowlist 는 신규 미봉인의 은신처)"
           % stale) if stale else "소멸·봉인·python 미사용 0")
    sealed_names = sorted((n for n, (py, s) in info.items() if py and s), key=str)
    check("ⓒ(ii) 봉인 술어 실증(cys-dept 봉인 인식)", "cys-dept" in sealed_names,
          "봉인 %s" % sealed_names)


# ═══════════════════════════════════════════════════════════════════════════
# ⓒ(iii) Rust python 직스폰 지점 — src/lib.rs bundled_python_spawn_sites_are_enumerated_and_sealed 미러
# ═══════════════════════════════════════════════════════════════════════════
RUST_SPAWN_PIN = {  # Rust 핀 expected 와 동일 값(2026-08-26 실측 · 2026-09-03 파이썬 미러 재실측 일치)
    "src/bin/cys.rs": 1,                  # 테스트(tar escape 재현 · 호스트 python) · 층3 바닥
    "src/bin/cysd/boot_supervisor.rs": 1,  # run_ensure_team · .env 직봉인
    "src/bin/cysd/main.rs": 2,             # office-bridge(tokio 직봉인) + self-test 게이트
    "src-tauri/src/main.rs": 4,            # orchestra/resource-gate/boot/org · inject_runtime_path
}
_ASCII_LOWER = str.maketrans(string.ascii_uppercase, string.ascii_lowercase)  # Rust to_ascii_lowercase 동형


def check_rust_spawn():
    needle = "Command" + "::" + "new("  # Rust 핀과 같은 조각 결합(자기참조 회피 관례 통일)
    files = []
    for base in (("src",), ("src-tauri", "src")):
        for dp, dns, fns in os.walk(os.path.join(ROOT, *base)):
            dns[:] = sorted((d for d in dns if d not in ("vendor", "target")), key=str)
            for fn in sorted(fns, key=str):
                if fn.endswith(".rs"):
                    files.append(os.path.join(dp, fn))
    check("ⓒ(iii) .rs 소스 트리 스캔(>10)", len(files) > 10,
          "files=%d%s" % (len(files), "" if len(files) > 10 else " — 측정 불능은 통과가 아니다"))
    found, bad_utf8 = {}, []
    for f in files:
        with open(f, "rb") as fh:
            raw = fh.read()
        try:
            src = raw.decode("utf-8")  # Rust read_to_string 동형(엄격) — rustc 도 비-UTF-8 소스를 거부한다
        except UnicodeDecodeError:
            bad_utf8.append(_rel(f))
            continue
        rel = _rel(f)
        for line in _lines(src):
            t = line.lstrip()
            if t.startswith("//"):
                continue
            pos = t.find(needle)
            if pos < 0:
                continue
            rest = t[pos + len(needle):]
            close = rest.find(")")
            if close < 0:
                continue
            arg = rest[:close].translate(_ASCII_LOWER)
            if "py" in arg or "python" in t.translate(_ASCII_LOWER):
                found[rel] = found.get(rel, 0) + 1
    check("ⓒ(iii) .rs UTF-8 판독", not bad_utf8,
          "비-UTF-8 %s — Rust 핀은 이 파일을 건너뛰지만 측정 불능은 통과가 아니다" % bad_utf8 if bad_utf8 else "전수 판독")
    fmt = lambda d: "{" + ", ".join("%s:%d" % kv for kv in sorted(d.items(), key=lambda kv: str(kv[0]))) + "}"
    check("ⓒ(iii) Rust python 직스폰 지점 2언어 미러 핀", found == RUST_SPAWN_PIN,
          "실측 %s · 핀 %s%s" % (fmt(found), fmt(RUST_SPAWN_PIN),
                                "" if found == RUST_SPAWN_PIN else
                                " — 새 지점이면 ① std 는 python_command 팩토리, tokio/기타 빌더는 "
                                ".env(cys::ENV_PY_NO_BYTECODE, cys::PY_NO_BYTECODE_ON), GUI 는 inject_runtime_path 로 "
                                "**먼저 봉인**하고 ② src/lib.rs 핀과 이 RUST_SPAWN_PIN 을 **같은 커밋**에서 갱신하라"
                                "(한쪽만 올리면 2언어 미러가 깨진 채 초록/적색이 갈린다)"))


def main():
    lib_rs = os.path.join(ROOT, "src", "lib.rs")
    if not os.path.isfile(lib_rs):
        print("UNMEASURED: repo root not found (root=%s · src/lib.rs 부재 = 팩 단독 설치) — "
              "측정 불능은 통과가 아니다(exit 2)" % ROOT)
        return 2
    print("census root: %s" % ROOT)
    check_enforcement()
    check_referencing()
    check_hooks()
    check_bin()
    check_rust_spawn()
    if fails:
        print("FAILED %d: %s" % (len(fails), "; ".join(fails)))
        return 1
    print("PYSEAL-CENSUS-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
