#!/usr/bin/env bash
# lane-parity-rehearsal.sh — 팩 스위트 **레인 대조 게이트의 로컬 예행**(0.14.31 통합 · CONTRACTS B-9).
#
# 왜 존재하는가: 레인 대조 게이트(`ci-branch.yml` 의 '팩 스위트 레인 대조' 스텝)는 브랜치 push
#   에서만 돈다. 그런데 신규 검체 등재는 **3완전 레인 동시 수정**이라 손이 미끄러지기 쉽고,
#   미끄러진 결과는 push 뒤 CI 로그에서야 보인다. 이 스크립트는 그 판정을 커밋 전에 낸다.
#
# ★로직을 복제하지 않는다. 이 리포가 반복해 당한 사고의 형태가 '목록·판정이 여러 파일에 손으로
#   복제돼 갈리는 것'이라, 예행 도구가 게이트를 베끼면 같은 계급의 결함을 하나 더 만드는 셈이다.
#   그래서 1단계는 `ci-branch.yml` 안의 게이트 파이썬을 **원본에서 추출해 그대로 실행**한다.
#   추출에 실패하면 조용히 통과하지 않고 중단한다(fail-closed · 워크플로 구조가 바뀐 신호).
#
# 2단계는 게이트가 **보지 않는 것**을 본다: 게이트는 '이름이 세 레인에 다 있는가' 만 묻고 그
#   이름의 **파일이 있는가**는 묻지 않는다(그 단언은 CI 런타임의 `[ -f "$f" ]` 에 있다). 여기서
#   미리 확인하고, 아직 머지되지 않아 없는 것은 PENDING_MERGE 에 근거와 함께 등재한다.
#
# 사용:
#   scripts/lane-parity-rehearsal.sh           # 예행(PENDING 은 통과 · 배너로 남김)
#   scripts/lane-parity-rehearsal.sh --strict  # 머지 뒤 검증(PENDING 이 남아 있으면 실패)
#
# 종료코드: 0=통과 · 1=계약 위반(등재 비대칭 · 등재됐는데 파일 없음) · 3=구조 판별 실패(도구 수리)
set -uo pipefail

STRICT=0
[ "${1:-}" = "--strict" ] && STRICT=1

cd "$(dirname "$0")/.."
CI_YML=".github/workflows/ci-branch.yml"
[ -f "$CI_YML" ] || { echo "::error::$CI_YML 없음 — 리포 루트에서 실행하라" >&2; exit 3; }

echo "── 1단계: 레인 대조 게이트(원본 추출 실행) ───────────────────────────────"
GATE_SRC="$(mktemp)"
trap 'rm -f "$GATE_SRC"' EXIT
python3 - "$CI_YML" "$GATE_SRC" <<'PYEXTRACT'
import sys, pathlib
yml, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
lines = yml.read_text(encoding="utf-8").splitlines()
def die(msg):
    print("::error::게이트 파이썬을 추출하지 못했다 — %s. 워크플로 구조가 바뀌었다면 이 "
          "추출기를 고쳐라(조용한 통과 금지 · exit 3)." % msg, file=sys.stderr)
    raise SystemExit(3)
# ★게이트 자신과 **같은 순차 주사**로 표식 구간을 잡는다(BEGIN 판정이 END 판정보다 앞).
#   두 표식이 한 줄에 같이 나오는 줄이 실제로 있다 — 게이트 소스의 `SELF_BEGIN, SELF_END = …`
#   대입문이다. 그 줄을 END 로 세는 순진한 계수는 구간을 조기에 닫는다(초안이 그렇게 틀렸다).
SB, SE = "LANE-GATE-SELF-BEGIN", "LANE-GATE-SELF-END"
b = next((i for i, l in enumerate(lines) if SB in l), None)
if b is None:
    die("%s 표식이 없다" % SB)
e = next((i for i in range(b + 1, len(lines)) if SE in lines[i] and SB not in lines[i]), None)
if e is None:
    die("%s 뒤에 %s 가 없다 — 표식이 열린 채 끝났다" % (SB, SE))
b, e = [b], [e]
heads = [i for i in range(b[0], e[0]) if lines[i].strip() == "python3 - <<'PY'"]
if len(heads) != 1:
    die("표식 사이의 `python3 - <<'PY'` 가 %d 개다(1 이어야 한다)" % len(heads))
s = heads[0]
ind = len(lines[s]) - len(lines[s].lstrip())
ends = [i for i in range(s + 1, e[0])
        if lines[i].strip() == "PY" and len(lines[i]) - len(lines[i].lstrip()) == ind]
if not ends:
    die("heredoc 종료 `PY` 를 표식 안에서 찾지 못했다")
body = lines[s + 1:ends[0]]
if not body:
    die("추출된 게이트 본문이 0행이다")
out.write_text("\n".join(l[ind:] if not l[:ind].strip() else l for l in body) + "\n",
               encoding="utf-8")
print("[추출] %s:%d-%d · %d행" % (yml, s + 2, ends[0], len(body)))
PYEXTRACT
rc=$?
[ $rc -eq 0 ] || exit $rc
python3 "$GATE_SRC"
GATE_RC=$?
if [ $GATE_RC -ne 0 ]; then
  echo "::error::레인 대조 실패(exit $GATE_RC) — 1=등재 비대칭 · 3=게이트 파서 파손" >&2
  exit $GATE_RC
fi

echo
echo "── 2단계: 등재된 이름의 파일 존재(게이트가 보지 않는 축) ─────────────────"
python3 - "$STRICT" <<'PYEXIST'
import os, re, sys

STRICT = sys.argv[1] == "1"
LANES = {
    "ci-branch":    ".github/workflows/ci-branch.yml",
    "release":      ".github/workflows/release.yml",
    "pack-release": ".github/workflows/pack-release.yml",
}
DIRS = ("cysjavis-pack/bin/tests", "scripts/tests")

# 글롭으로 도는 이름 — 추출 정규식이 `*` 앞에서 끊겨 **접두 토큰**이 된다. 파일 1개 이상이
# 글롭에 걸리면 해소된 것으로 본다(0 개면 글롭 스텝이 빈 루프를 도는 것이므로 실패다).
GLOB_TOKENS = {"test_phoenix_": "cysjavis-pack/bin/tests/test_phoenix_*.py"}

# 이름은 등재됐으나 **파일이 아직 이 브랜치에 없는** 것. 값 = (도착 경로, 근거).
# ★등재는 왜 먼저 하는가: 3완전 레인 동시 등재가 계약(CONTRACTS B-9)이고, 등재를 머지 뒤로
#   미루면 그 커밋이 다시 '레인 하나 빼먹기' 의 기회가 된다. 파일이 도착하면 여기서 지워라.
# ★2026-09-08 통합 완료 — 위 10종은 `wp/0.14.31-pack` 머지(76a83ff)로 전부 도착했다.
#   도착한 이름을 여기 남겨 두면 `--strict` 가 아닌 예행에서 그 이름들이 "대기"로 접혀
#   존재 축이 사실상 꺼진다(그래서 게이트가 도착 시 ::warning:: 로 청소를 재촉한다).
#   다음 통합에서 다시 쓸 때는 {이름: (도착 브랜치, 근거)} 형태로 채운다.
PENDING_MERGE = {}

SB, SE = "LANE-GATE-SELF-BEGIN", "LANE-GATE-SELF-END"

def names(path):
    found, skip = set(), False
    for line in open(path, encoding="utf-8").read().splitlines():
        if SB in line:
            skip = True; continue
        if SE in line:
            skip = False; continue
        if skip:
            continue
        found |= set(re.findall(r"\btest_[a-z0-9_]+", re.sub(r"#.*$", "", line)))
    return found

union = set()
for p in LANES.values():
    union |= names(p)
if not union:
    print("::error::세 레인에서 이름 0건 — 추출기 파손(fail-closed).", file=sys.stderr)
    sys.exit(3)

import glob as _g
ok, pending, missing, stale = [], [], [], []
for n in sorted(union):
    if n in GLOB_TOKENS:
        if _g.glob(GLOB_TOKENS[n]):
            ok.append(n)
        else:
            missing.append((n, "글롭 %s 가 0건" % GLOB_TOKENS[n]))
        continue
    hit = next((os.path.join(d, n + ".py") for d in DIRS
                if os.path.exists(os.path.join(d, n + ".py"))), None)
    if hit:
        ok.append(n)
        if n in PENDING_MERGE:
            stale.append((n, hit))
    elif n in PENDING_MERGE:
        pending.append(n)
    else:
        missing.append((n, "%s 어디에도 <이름>.py 없음" % " · ".join(DIRS)))

print("[존재 대조] 등재 %d종 · 파일 확인 %d · 머지 대기 %d · 미해소 %d"
      % (len(union), len(ok), len(pending), len(missing)))
for n, hit in stale:
    print("::warning::'%s' 파일이 도착했다(%s) — PENDING_MERGE 에서 제거하라" % (n, hit))
for n in pending:
    br, why = PENDING_MERGE[n]
    print("  대기 %-34s ← %s (%s)" % (n, br, why))
for n, why in missing:
    print("::error::  미해소 %s — %s" % (n, why), file=sys.stderr)

if missing:
    print("::error::등재된 이름의 파일이 없다 — CI 런타임의 `[ -f \"$f\" ]` 단언이 붉어진다. "
          "파일을 커밋하거나(git add 누락) PENDING_MERGE 에 근거와 함께 등재하라.",
          file=sys.stderr)
    sys.exit(1)
if pending and STRICT:
    print("::error::--strict 인데 머지 대기 %d종이 남아 있다 — 팩 브랜치 머지가 끝나지 않았거나 "
          "PENDING_MERGE 를 청소하지 않았다." % len(pending), file=sys.stderr)
    sys.exit(1)
if pending:
    print("\n[예행 판정] 레인 대조 통과 · 파일 존재는 머지 대기 %d종을 제외하고 통과."
          "\n            머지 후 `--strict` 로 다시 돌려라(그때 0 이어야 완결)." % len(pending))
else:
    print("\n[예행 판정] 레인 대조 통과 · 등재 전건 파일 확인.")
PYEXIST
