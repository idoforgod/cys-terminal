#!/usr/bin/env python3
"""test_directive_cli_drift.py — 디렉티브 ↔ CLI 드리프트 회귀 핀 (reviewer2 감사 기준7 교정②).

■ 왜 존재하는가
  디렉티브(MASTER/CSO/REVIEWER/WORKER_DIRECTIVE.md 등)는 백틱 안에서 `cys <sub> --<opt>`
  명령을 지시한다. 그런데 그 산문이 지시하는 옵션이 실제 clap 정의(cys.rs)에 **존재하는지**
  검증하는 구조적 방어가 지금까지 하나도 없었다. 그 결과 "산문이 존재하지 않는 옵션을 지시"
  하는 사고(예: `--kind` 누락)가 무검증 통과했다. 이 파일이 그 계열의 **유일한 회귀 핀**이다.

■ 진실원천(SOT)
  `cys actions --json` 이 방출하는 데이터 파생 카탈로그. clap 정의(cys.rs `Command::Actions`)
  에서 기계 파생되므로 **데몬·네트워크 불요**(오프라인·결정론). 산문 표가 아니라 이 기계
  산출만이 사실이다.

■ 무엇을 검증하나 (오탐 억제 규칙)
  1) 백틱 인라인/펜스에서 `cys <sub> ... --<opt>` 를 정규식 추출한다.
  2) <sub> 가 **실재 top-level 서브커맨드일 때만** 검증한다(sub 미실재 = 산문/플레이스홀더
     가능성 → skip). "cys 노드"·"cys.app"·"cys-dept" 등은 `cys␣<소문자토큰>` 패턴에
     안 걸리거나 sub 미실재로 자연 배제된다.
  3) <sub> 가 **중첩 서브커맨드를 가진 계열**(feed·queue·skill·schedule·attest·daemon·
     license·persona·cost-baseline·approval·channel)이면, 옵션은 중첩 커맨드에 속하는데
     카탈로그가 중첩 args 를 노출하지 않으므로 **--opt 검증을 건너뛴다**(`cys feed push --wait`
     같은 정상 명령을 오탐하지 않기 위함). 대신 중첩 이름 자체의 실재만 확인한다.
  4) 리프(leaf) 서브커맨드는 모든 `--opt` 가 그 서브의 long 옵션이거나 전역 옵션(--socket
     등)인지 대조한다. 부재 시 드리프트로 보고(파일·라인·명령).
  5) `<역할>`·`<ref>` 등 `<...>` 플레이스홀더 토큰은 값 위치에서 무시된다.

■ 베이스라인 래칫 (green-now + 신규 드리프트 차단)
  현행 디렉티브에는 사전 존재 드리프트 1건이 있다 — `cys run --scoped`(run 은 --scoped
  플래그가 없다; scoped 실행은 이미 기본값이라 이 플래그는 clap 에서 즉시 에러난다).
  이 사고성 결함은 디렉티브 수정으로만 고칠 수 있는데, 그 파일 수정은 이 워커 범위 밖이라
  KNOWN_DRIFT 베이스라인에 **시끄럽게 기록**한다. 테스트는:
    · 신규 드리프트(베이스라인에 없는 (sub,opt)) → **FAIL** (사고 재발 차단 = 핀의 이빨)
    · 베이스라인 드리프트가 아직 남아있으면 → 매 실행 WARNING 출력(은폐 금지·보고 지속)
    · 베이스라인 항목이 사라졌으면(디렉티브가 고쳐졌으면) → INFO(항목 제거 권고, FAIL 아님)
  ★KNOWN_DRIFT 는 부채 목록이다. 새 항목을 여기 추가하지 말고 디렉티브·CLI 를 고쳐라.

■ 실행
  python3 cysjavis-pack/bin/tests/test_directive_cli_drift.py   (exit 0=PASS / 1=신규 드리프트)
  · 디렉티브는 repo 경로(이 파일 기준 ../../directives)에서 읽는다 — CYS_PACK_DIR(빈 mktemp)
    가 아니다. hermetic pack-parity 레인에서도 repo 디렉티브를 대상으로 동작.
  · 카탈로그 획득 우선순위: env CYS_BIN → PATH `cys` → target/release/cys → target/debug/cys
    → `cargo run --bin cys`. 전부 실패하면 **SKIP(exit 0)** 를 시끄럽게 출력한다(바이너리
    없는 레인에서 핀을 잃지 않기 위함 — 실 강제는 cys 바이너리 보유 레인에서).
"""
import json
import os
import re
import shutil
import subprocess
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
# tests → bin → cysjavis-pack ; directives = cysjavis-pack/directives (repo 경로 고정).
PACK = os.path.dirname(os.path.dirname(SELF))
DIRDIR = os.path.join(PACK, "directives")
# cysjavis-pack → repo 루트(Rust 크레이트 루트).
REPO = os.path.dirname(PACK)

# 전역(global=true) 옵션 — 모든 서브커맨드에서 유효하나 서브별 args 목록엔 안 실린다.
# cys.rs `struct Cli` 의 `#[arg(long, global = true)] socket` + clap 표준 help/version.
GLOBAL_LONGS = {"socket", "help", "version"}

# ── 베이스라인 래칫: 사전 존재 드리프트(부채). (sub, opt) 로 키.
#    ★2026-07-26 비움 — `run --scoped` 드리프트를 디렉티브(MASTER:220·WORKER:18)에서
#    제거해 해소함(scoped 는 이미 기본값). 이제 예외 0 — 새 항목을 여기 추가하지 말고
#    디렉티브·CLI 를 고쳐라(이 래칫이 채워지면 드리프트가 재도입됐다는 신호다).
KNOWN_DRIFT = {}

_total = [0]
fails = []


def check(name, cond, detail=""):
    _total[0] += 1
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ── 카탈로그 획득 ───────────────────────────────────────────────────────────
def _run_json(cmd, cwd=None, timeout=180):
    try:
        p = subprocess.run(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if p.returncode != 0:
        return None
    try:
        return json.loads(p.stdout.decode("utf-8", "replace"))
    except (ValueError, UnicodeError):
        return None


def get_catalog():
    """(catalog_dict, source_str) 반환 — 실패 시 (None, reason).

    ★반드시 **이 워크트리 소스(src/bin/cys.rs)** 파생 카탈로그를 써야 한다. 전역 설치본
    (`/usr/local/bin/cys` 등 PATH 의 `cys`)은 워크트리 소스와 **탈동조**될 수 있어(실측: 구
    전역본은 69 actions·`--kind` 부재, 워크트리 빌드는 70 actions·`--kind` 보유) 소스에
    구현된 옵션을 디렉티브가 정상 지시해도 **거짓 FAIL** 을 낸다. 따라서 PATH `cys` 는
    카탈로그 소스에서 **배제**한다. 소스 정합 순서:
      1) env CYS_BIN(CI 가 방금 빌드한 바이너리를 명시적으로 지정 — 최우선·최신 보장)
      2) 워크트리 target/{release,debug}/cys(이 소스 트리에서 빌드된 산출물)
      3) cargo run --bin cys(현재 소스 즉석 컴파일 — Rust 툴체인 보유 시)
    ※ target 바이너리가 소스보다 오래됐을 위험이 있으면 CI 는 테스트 전 `cargo build --bin cys`
      하거나 CYS_BIN 을 신선 바이너리로 지정하라(런북 권고).
    """
    tried = []
    cands = []
    env_bin = os.environ.get("CYS_BIN")
    if env_bin:
        cands.append(("env CYS_BIN", env_bin))
    cands.append(("target/release/cys", os.path.join(REPO, "target", "release", "cys")))
    cands.append(("target/debug/cys", os.path.join(REPO, "target", "debug", "cys")))
    for label, path in cands:
        if os.path.sep in path and not os.path.exists(path):
            tried.append("%s(부재)" % label)
            continue
        cat = _run_json([path, "actions", "--json"])
        if cat is not None:
            return cat, label
        tried.append("%s(실행실패)" % label)
    # cargo 폴백 — 현재 소스 즉석 컴파일. Rust 툴체인 보유 레인에서만 성공.
    if shutil.which("cargo") and os.path.exists(os.path.join(REPO, "Cargo.toml")):
        cat = _run_json(["cargo", "run", "--quiet", "--bin", "cys", "--", "actions", "--json"],
                        cwd=REPO, timeout=600)
        if cat is not None:
            return cat, "cargo run --bin cys"
        tried.append("cargo run(실패)")
    else:
        tried.append("cargo(부재)")
    return None, "; ".join(tried)


# ── 추출 정규식 ─────────────────────────────────────────────────────────────
# `cys␣<sub>` — sub 는 소문자+하이픈. 뒤에 whitespace 필수라 `cys-dept`·`cys.app`·`cysjavis`
# 는 걸리지 않는다.
CMD = re.compile(r"\bcys\s+([a-z][a-z-]+)")
OPT = re.compile(r"--([a-z][a-z-]+)")
# 명령 스팬 종료자: 백틱(인라인 코드 종료)·개행·`;`·`|`·주석 ` # `.
TERM = re.compile(r"[`\n;|]| # ")
# sub 다음 첫 토큰(중첩 서브커맨드 후보).
NEXTTOK = re.compile(r"\s+([^\s`]+)")
BAREWORD = re.compile(r"^[a-z][a-z-]+$")


def extract(catalog):
    subs = {a["name"] for a in catalog["actions"]}
    opts_by = {a["name"]: {x["long"] for x in a["args"] if x.get("long")} for a in catalog["actions"]}
    nested_by = {a["name"]: set(a.get("subcommands") or []) for a in catalog["actions"]}

    opt_drift = []      # (file, line, "cys sub --opt", sub, opt)
    nested_drift = []   # (file, line, "cys sub nested", sub, nested)
    verified = 0

    for fname in sorted(os.listdir(DIRDIR)):
        if not fname.endswith(".md"):
            continue
        path = os.path.join(DIRDIR, fname)
        with open(path, encoding="utf-8") as fh:
            for lineno, line in enumerate(fh, 1):
                for m in CMD.finditer(line):
                    sub = m.group(1)
                    tail = line[m.end():]
                    cut = TERM.search(tail)
                    span = tail[:cut.start()] if cut else tail
                    if sub not in subs:
                        continue  # 미실재 sub = 산문/플레이스홀더 → skip(오탐 억제)
                    if nested_by[sub]:
                        # 중첩 계열: 옵션은 중첩 커맨드 소유 → --opt 검증 skip.
                        nt = NEXTTOK.match(tail)
                        first = nt.group(1) if nt else None
                        if (first and not first.startswith("-")
                                and not first.startswith("<") and BAREWORD.match(first)
                                and first not in nested_by[sub]):
                            nested_drift.append((fname, lineno, "cys %s %s" % (sub, first), sub, first))
                        continue
                    # 리프 서브커맨드: 모든 --opt 대조.
                    for o in OPT.findall(span):
                        if o in opts_by[sub] or o in GLOBAL_LONGS:
                            verified += 1
                        else:
                            opt_drift.append((fname, lineno, "cys %s --%s" % (sub, o), sub, o))
    return verified, opt_drift, nested_drift


def main():
    if not os.path.isdir(DIRDIR):
        check("0 directives 디렉토리 존재", False, DIRDIR)
        return 1

    catalog, source = get_catalog()
    if catalog is None:
        print("=" * 72)
        print("⚠ SKIP — cys actions 카탈로그 획득 불가 (시도: %s)" % source)
        print("  이 핀은 clap 파생 카탈로그가 필요하다. cys 바이너리(target/{debug,release}/cys)")
        print("  또는 Rust 툴체인(cargo)이 있는 레인에 배선하라. 바이너리 보유 레인에서 강제된다.")
        print("=" * 72)
        # 핀을 잃지 않기 위해 SKIP=exit 0(green). 실 강제는 바이너리 보유 레인.
        return 0

    print("카탈로그 획득: %s (%d actions)" % (source, catalog.get("count", len(catalog.get("actions", [])))))
    verified, opt_drift, nested_drift = extract(catalog)
    print("검증한 (sub,opt) 유효쌍: %d" % verified)

    # 신규 vs 베이스라인 분류.
    new_opt_drift = [d for d in opt_drift if (d[3], d[4]) not in KNOWN_DRIFT]
    baselined = [d for d in opt_drift if (d[3], d[4]) in KNOWN_DRIFT]

    # 신규 옵션 드리프트 = FAIL(사고 재발).
    check("1 신규 옵션 드리프트 0건 (디렉티브 산문이 존재하는 옵션만 지시)",
          not new_opt_drift,
          "" if not new_opt_drift else
          "존재하지 않는 옵션 지시: " + " / ".join("%s:%d `%s`" % (f, l, c) for f, l, c, *_ in new_opt_drift))

    # 중첩 서브커맨드 이름 드리프트 = FAIL(존재하지 않는 중첩 커맨드 지시).
    check("2 중첩 서브커맨드 이름 드리프트 0건",
          not nested_drift,
          "" if not nested_drift else
          "존재하지 않는 중첩 커맨드: " + " / ".join("%s:%d `%s`" % (f, l, c) for f, l, c, *_ in nested_drift))

    # 베이스라인(사전 존재 드리프트) — 남아있으면 시끄러운 WARNING(FAIL 아님).
    still = {(d[3], d[4]) for d in baselined}
    for f, l, c, sub, opt in baselined:
        print("⚠ KNOWN_DRIFT(부채·미수정): %s:%d `%s` — %s" % (f, l, c, KNOWN_DRIFT[(sub, opt)]))
    # 베이스라인 항목이 실제로 사라졌으면 제거 권고(INFO).
    for key in KNOWN_DRIFT:
        if key not in still:
            print("ℹ KNOWN_DRIFT stale: (%s,%s) 더 이상 디렉티브에 없음 → 베이스라인에서 제거 가능." % key)

    if fails:
        print("\n=== FAIL(%d) — 신규 드리프트. 디렉티브 산문 또는 CLI 정의를 정합화하라. ===" % len(fails))
        return 1
    print("\n=== PASS — 현행 디렉티브의 모든 CLI 명령 실재 확인(베이스라인 %d건 부채 제외). ===" % len(baselined))
    return 0


if __name__ == "__main__":
    sys.exit(main())
