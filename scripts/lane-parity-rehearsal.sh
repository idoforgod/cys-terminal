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
# 3단계(★D3 · 반성 라운드 2026-09-10)는 **역방향**이다: 디스크의 `test_*.py` 가 세 레인 union 에
#   있는가. 게이트는 레인 **간** 대칭만 재므로 세 레인 **모두**에 없는 파일은 union 밖이라 비대칭
#   0 으로 초록이었다 — 수용 검체 4종(session_start_hook·formation_gate_label·
#   review_prompt_verdict_path·dept_teardown_atomicity)이 그렇게 3레인 0회 실행이었고, 전체로는
#   34종이 어느 레인에도 없었다. 미등재는 UNREGISTERED_OK 에 **사유와 함께** 등재된 것만 통과한다
#   (게이트의 ALLOWED 와 같은 마찰 — "등재를 미룬다" 는 사유가 아니다). 이 축은 ci-branch 의
#   '레인 예행 도구' 스텝이 게이트로 돌린다.
#
# 4단계(★D12 · 반성 라운드 2026-09-10)는 **문서**를 본다: 릴리스 노트가 백틱 안에서 이름 붙인
#   저장소 상대 경로가 실재하는가. 릴리스 노트가 재측정 수집 도구를 `tools/queue_remeasure.py` 로
#   안내했는데 저장소에 `tools/` 자체가 없었다 — 담당자가 도구를 못 찾으면 임의 집계로 대체하거나
#   측정을 건너뛴다. 대상 문서는 `LANE_PARITY_DOCS`(os.pathsep 구분 · 기본값 = 릴리스 노트 1개)다.
#
# 사용:
#   scripts/lane-parity-rehearsal.sh             # 예행(PENDING 은 통과 · 배너로 남김)
#   scripts/lane-parity-rehearsal.sh --strict    # 머지 뒤 검증(PENDING 이 남아 있으면 실패)
#   scripts/lane-parity-rehearsal.sh --self-test # 자기 검체 — 3·4단계가 실제로 잡는가(양성·음성 대조)
#
# 종료코드: 0=통과 · 1=계약 위반(등재 비대칭 · 등재됐는데 파일 없음 · 사유 없는 미등재 파일 ·
#          사유 없는 부재 경로 인용) · 3=구조 판별 실패 또는 **잴 대상 0건**(도구 수리 · 조용한 초록 금지)
set -uo pipefail

STRICT=0
SELF_TEST=0
for arg in "$@"; do
  case "$arg" in
    --strict) STRICT=1 ;;
    --self-test) SELF_TEST=1 ;;
    *) echo "::error::모르는 인자: $arg (--strict | --self-test)" >&2; exit 3 ;;
  esac
done

cd "$(dirname "$0")/.."
CI_YML=".github/workflows/ci-branch.yml"
[ -f "$CI_YML" ] || { echo "::error::$CI_YML 없음 — 리포 루트에서 실행하라" >&2; exit 3; }

echo "── 1단계: 레인 대조 게이트(원본 추출 실행) ───────────────────────────────"
GATE_SRC="$(mktemp)"
SELF_TMP=""
trap 'rm -f "$GATE_SRC"; [ -n "$SELF_TMP" ] && rm -rf "$SELF_TMP"' EXIT
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

# 2·3단계 — 한 파이썬 블록이다(이름 추출기 `names()` 를 두 축이 공유한다 · 복제 금지).
#   env `LANE_PARITY_DIRS`(os.pathsep 구분)로 검체 디렉터리를 바꿀 수 있다 — 자기 검체가 임시
#   디렉터리를 **덧붙여** 3단계가 미등재 파일을 잡는지 재는 데 쓴다(리포 트리 무접촉).
existence_axes() {
python3 - "$STRICT" <<'PYEXIST'
import glob as _g
import os, re, sys

STRICT = sys.argv[1] == "1"
LANES = {
    "ci-branch":    ".github/workflows/ci-branch.yml",
    "release":      ".github/workflows/release.yml",
    "pack-release": ".github/workflows/pack-release.yml",
}
DIRS = tuple(d for d in os.environ.get(
    "LANE_PARITY_DIRS", os.pathsep.join(("cysjavis-pack/bin/tests", "scripts/tests"))
).split(os.pathsep) if d)

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

# ★3단계(D3) 역방향 축의 허용 목록 — 디스크에 있으나 세 레인 어디에도 등재되지 않은 파일.
#   값 = 사유. **사유 없는 등재 금지**(게이트 ALLOWED 와 같은 규율). "나중에 편입" 은 사유가
#   아니다 — 그 파일이 왜 CI 밖이어도 되는지, 아니면 무엇이 편입을 막는지를 적어라.
#   ★기준선(2026-09-10 · 반성 라운드 D3): 아래 30종은 0.14.31 **이전부터** 0레인이던 격차다
#   (integration-notes §7-3 · 로컬 전수 rc=0 · 등재만 없다). 이번 판은 수용 검체 4종만 편입했고
#   나머지는 "안 도는 검체는 게이트가 아니다" 계급의 **잔여 격차**로 여기 못박는다 — 편입은
#   다음 판의 독립 작업이고, 편입하는 커밋이 이 항목을 지운다(그때 이 축이 ::warning:: 으로
#   청소를 재촉한다). 이 사유는 "정당한 무관함" 이 아니라 **미해소의 기록**이다.
_BASELINE = ("0.14.31 이전부터 0레인이던 격차의 기준선 등재(2026-09-10 D3 · integration-notes §7-3 · "
             "로컬 rc=0) — 정당한 무관함이 아니라 미해소 기록 · 편입 커밋이 이 항목을 지운다")
UNREGISTERED_OK = {n: _BASELINE for n in (
    "test_atomic_bundle", "test_ceo_pending_gate", "test_cli_probe", "test_completion_guard_notice",
    "test_contracts_ct", "test_deploy_gate_bundle_swap", "test_dept_creds_seed", "test_dept_doctrine_v1",
    "test_dept_list_unregistered", "test_dept_ticket_deficit_zero", "test_dept_ticket_request",
    "test_distill_fx", "test_formation", "test_hud_bridge_master_idle", "test_installer_atomic",
    "test_lane_isolation_v1", "test_memory_desc_drift", "test_mission_boot_command_filter",
    "test_mission_harness_filter", "test_orchestra_ticket_snapshot", "test_orchestra_todo_path",
    "test_org_audit", "test_pack_syntax_warnings", "test_preflight_nlm_pin", "test_preflight_phase1_checks",
    "test_release_verify", "test_seat_revival", "test_verify_gate", "test_vibecheck", "test_viberoute",
)}

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

print("── 2단계: 등재된 이름의 파일 존재(게이트가 보지 않는 축) ─────────────────")
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

print()
print("── 3단계: 역방향 — 디스크의 검체가 세 레인 union 에 있는가(D3) ─────────────")
disk = {}
for d in DIRS:
    for p in _g.glob(os.path.join(d, "test_*.py")):
        disk[os.path.basename(p)[:-3]] = p
if not disk:
    print("::error::검체 디렉터리 %s 에서 test_*.py 0건 — 역방향 축이 잴 대상이 없다(fail-closed)."
          % " · ".join(DIRS), file=sys.stderr)
    sys.exit(3)

def registered(n):
    return n in union or any(n.startswith(tok) for tok in GLOB_TOKENS if tok in union)

unregistered = sorted(n for n in disk if not registered(n))
listed = [n for n in unregistered if n in UNREGISTERED_OK]
orphans = [n for n in unregistered if n not in UNREGISTERED_OK]
# 허용 목록이 낡았는가 — 편입됐거나 삭제된 이름은 경고(막을 이유는 없지만 방치하면 목록이 썩는다).
for n in sorted(UNREGISTERED_OK):
    if n not in disk:
        print("::warning::UNREGISTERED_OK '%s' 의 파일이 없다 — 삭제됐다면 목록에서도 지워라" % n)
    elif registered(n):
        print("::warning::UNREGISTERED_OK '%s' 이 이제 레인에 등재됐다 — 목록에서 지워라" % n)
print("[역방향] 디스크 %d종 · 등재 %d · 사유 있는 미등재 %d · 사유 없는 미등재 %d"
      % (len(disk), len(disk) - len(unregistered), len(listed), len(orphans)))
for n in orphans:
    print("::error::  미등재 %s (%s) — 세 레인 어디에도 없다. 3완전 레인에 같은 커밋으로 등재하거나, "
          "CI 밖이어도 되는 **사유**를 UNREGISTERED_OK 에 적어라(\"나중에\" 는 사유가 아니다)"
          % (n, disk[n]), file=sys.stderr)

print()
print("── 4단계: 릴리스 노트가 이름 붙인 저장소 경로의 실재(D12) ────────────────────")
# ★D12(반성 라운드 2026-09-10): 릴리스 노트가 재측정 수집 도구를 `tools/queue_remeasure.py` 로
#   안내했는데 저장소에 `tools/` 자체가 없었다 — 담당자가 도구를 못 찾으면 임의 집계로 대체하거나
#   측정을 건너뛴다(§9 WP-5 '재측정 보고 선행' 붕괴). 이 축은 그 형태 **하나만** 판정한다:
#   문서가 백틱 안에서 이름 붙인 **저장소 상대 경로가 실재하는가**.
#
# 추출 규약(거짓 양성을 만들지 않기 위한 보수적 규칙 · codex 설계 검토 반영):
#   · 백틱 인라인 span 을 공백으로 쪼갠 **토큰의 맨 앞**이 접두로 시작할 때만 후보다. 그래서
#     `~/…/tools/x.py`(저장소 밖 · 토큰이 `~` 로 시작) · `git show v0.14.30:src/bin/cys.rs`
#     (과거 태그 트리 참조 · 토큰이 `v0.14.30:` 로 시작)는 후보가 아니다 — 저장소 밖·과거 트리를
#     **정확히 설명한 문장**을 붉히지 않는다.
#   · 경로 문자는 `[A-Za-z0-9._/+-]` 까지다 — 한국어 조사·괄호에서 끊긴다(`scripts/x.sh를 실행`).
#   · 접두 뒤가 비면 단일 경로가 아니므로 '서식 인용' 으로 세기만 한다(`tools/` 디렉터리 언급 ·
#     `docs/*.md` 글롭 · `scripts/{a,b}.sh` · `docs/<이름>.md` 자리표시자).
#   · `hooks/…` **만** 팩 상대 표기이므로 `cysjavis-pack/` 폴백을 준다. 다른 접두에 폴백을 주면
#     `scripts/x.py` 가 팩 안에만 있을 때 **틀린 실행 경로**를 정상으로 인정한다(codex).
#
# 이 축이 재지 못하는 것(정직한 한계 — 적어 두지 않으면 다음 사람이 보증으로 읽는다): 백틱 밖
#   평문·마크다운 링크·`$VAR/…` 변수 표기·**파일명만** 적은 인용은 후보가 아니고, 파일이 있어도
#   그것이 **배포 팩에 실렸는지**·안내한 옵션을 지원하는지는 `os.path.exists` 가 증명하지 못한다.
#   이 축은 '안내가 옳다' 의 증명이 아니라 **D12 형태(실재하지 않는 저장소 경로 안내)의 재발 차단**이다.
#
# 왜 릴리스 노트 1개인가: 다른 docs 는 이 레인의 소유 밖이다 — 고칠 권한이 없는 문서의 과거·예시
#   경로로 3레인을 막으면 복구 책임과 권한이 갈린다(codex). 확대는 소유자별 정리 뒤에 한다.
DOCS = [d for d in os.environ.get(
    "LANE_PARITY_DOCS", "docs/RELEASE_NOTES_0.14.32.md").split(os.pathsep) if d]
DOC_PREFIXES = ("scripts/", "tools/", "src/", "docs/", "hooks/", "cysjavis-pack/")
PACK_FALLBACK_PREFIX = "hooks/"      # 팩 상대 표기는 이것뿐이다(codex: 폴백을 넓히지 마라)
# 의도적으로 실재하지 않는 경로의 허용 목록 — 값 = 사유(게이트 ALLOWED · UNREGISTERED_OK 와 같은
#   마찰). "나중에 넣는다" 는 사유가 아니다. 실측 2026-09-10: 부재 0 이라 비어 있다.
DOC_PATH_ALLOWED = {}

def doc_candidates(text):
    """(검사 후보, 서식 인용) — 위 추출 규약 그대로."""
    checked, formatted = [], []
    for span in re.findall(r"`([^`\n]+)`", text):
        for tok in span.split():
            m = re.match(r"[A-Za-z0-9._/+-]+", tok)
            if not m:
                continue
            cand = m.group(0)
            pre = next((p for p in DOC_PREFIXES if cand.startswith(p)), None)
            if pre is None:
                continue
            (formatted if len(cand) == len(pre) else checked).append(cand)
    return checked, formatted

doc_missing, doc_checked, doc_formatted, doc_allowed = [], 0, 0, 0
for doc in DOCS:
    if not os.path.exists(doc):
        print("::error::4단계 대상 문서가 없다: %s (LANE_PARITY_DOCS 를 확인하라)" % doc,
              file=sys.stderr)
        sys.exit(3)
    checked, formatted = doc_candidates(open(doc, encoding="utf-8").read())
    doc_formatted += len(formatted)
    for rel in checked:
        doc_checked += 1
        probe = [rel] + ([os.path.join("cysjavis-pack", rel)]
                         if rel.startswith(PACK_FALLBACK_PREFIX) else [])
        if any(os.path.exists(c) for c in probe):
            continue
        if rel in DOC_PATH_ALLOWED:
            doc_allowed += 1
            continue
        doc_missing.append((doc, rel))
print("[문서 경로] %s · 검사 %d건 · 서식 인용 %d · 사유 있는 부재 %d · 사유 없는 부재 %d"
      % (" · ".join(DOCS), doc_checked, doc_formatted, doc_allowed, len(doc_missing)))
for rel in sorted(DOC_PATH_ALLOWED):
    if os.path.exists(rel):
        print("::warning::DOC_PATH_ALLOWED '%s' 이 이제 실재한다 — 목록에서 지워라" % rel)
if doc_checked == 0:
    print("::error::4단계가 잰 경로가 **0건**이다 — 문서가 저장소 경로 안내를 잃었거나 추출기가 "
          "파손됐다. 0건은 초록이 아니다(D10 과 같은 규율 · 잴 대상이 없으면 게이트가 아니다).",
          file=sys.stderr)
    sys.exit(3)
for doc, rel in doc_missing:
    print("::error::  %s 가 인용한 `%s` 가 저장소에 없다 — 실재하는 경로로 고치거나, 저장소 밖임을 "
          "문장으로 밝히거나(백틱 안에 저장소 상대 경로로 적지 마라), DOC_PATH_ALLOWED 에 사유와 "
          "함께 등재하라" % (doc, rel), file=sys.stderr)

if missing:
    print("::error::등재된 이름의 파일이 없다 — CI 런타임의 `[ -f \"$f\" ]` 단언이 붉어진다. "
          "파일을 커밋하거나(git add 누락) PENDING_MERGE 에 근거와 함께 등재하라.",
          file=sys.stderr)
    sys.exit(1)
if orphans:
    print("::error::세 레인 모두에 없는 검체는 레인 대조 게이트의 union 밖이라 **비대칭 0 으로 초록**"
          "이다 — 안 도는 검체는 게이트가 아니다.", file=sys.stderr)
    sys.exit(1)
if doc_missing:
    print("::error::릴리스 노트가 실재하지 않는 저장소 경로를 안내한다(D12) — 재측정 담당자가 "
          "도구를 못 찾으면 임의 집계로 대체하거나 측정을 건너뛴다.", file=sys.stderr)
    sys.exit(1)
if pending and STRICT:
    print("::error::--strict 인데 머지 대기 %d종이 남아 있다 — 팩 브랜치 머지가 끝나지 않았거나 "
          "PENDING_MERGE 를 청소하지 않았다." % len(pending), file=sys.stderr)
    sys.exit(1)
if pending:
    print("\n[예행 판정] 레인 대조 통과 · 파일 존재는 머지 대기 %d종을 제외하고 통과 · 역방향 통과 "
          "· 문서 경로 실재 통과."
          "\n            머지 후 `--strict` 로 다시 돌려라(그때 0 이어야 완결)." % len(pending))
else:
    print("\n[예행 판정] 레인 대조 통과 · 등재 전건 파일 확인 · 역방향(사유 없는 미등재 0) 통과 "
          "· 문서 경로 실재(사유 없는 부재 0) 통과.")
PYEXIST
}

echo
existence_axes
AX_RC=$?
[ $AX_RC -eq 0 ] || exit $AX_RC

if [ $SELF_TEST -eq 1 ]; then
  echo
  echo "── 자기 검체: 3단계가 임의 미등재 파일을 실제로 잡는가(음성 대조) ────────────"
  # 리포 트리에 쓰지 않는다 — 임시 디렉터리를 검체 디렉터리 목록에 **덧붙여** 미등재 파일 하나를
  # 보인다. 통과 대조(위 existence_axes 의 rc=0)가 있으므로 이 실패 대조가 없으면 3단계는
  # "다 허용해서 초록" 으로도 만족된다.
  SELF_TMP="$(mktemp -d)"
  PROBE="test_zz_probe_unregistered"
  : > "$SELF_TMP/$PROBE.py"
  SELF_LOG="$SELF_TMP/reverse.log"
  LANE_PARITY_DIRS="cysjavis-pack/bin/tests:scripts/tests:$SELF_TMP" existence_axes > "$SELF_LOG" 2>&1
  PROBE_RC=$?
  if [ $PROBE_RC -ne 1 ]; then
    cat "$SELF_LOG"
    echo "::error::자기 검체 실패 — 미등재 파일 $PROBE.py 를 넣었는데 3단계가 exit 1 이 아니라 exit $PROBE_RC 를 냈다(역방향 축이 눈을 감았다)" >&2
    exit 1
  fi
  if ! grep -q "미등재 $PROBE " "$SELF_LOG"; then
    cat "$SELF_LOG"
    echo "::error::자기 검체 실패 — exit 1 이지만 그 사유가 $PROBE 미등재가 아니다(다른 이유로 붉어졌다)" >&2
    exit 1
  fi
  echo "[자기 검체] 미등재 $PROBE.py → exit 1 · 사유 일치 (역방향 축 살아 있음)"

  # ── 4단계(D12) 대조 3종 — 축이 살아 있고, 정상 문서를 붉히지 않고, 0건이 초록이 아니다 ──
  #   음성 대조만 있으면 "다 붉혀서" 도 만족되고, 양성 대조만 있으면 "다 통과시켜서" 도 만족된다.
  #   rc 도 서로 다르다(부재=1 · 0건=3) — "exit 1 이면 4단계다" 로 읽는 오판을 막는다(codex).
  DOC_DIR="$SELF_TMP/docs4"
  mkdir -p "$DOC_DIR"

  # ① 양성 대조: 한국어 조사·글롭·자리표시자·저장소 밖 절대경로·과거 태그 참조·디렉터리 언급이
  #    섞인 **정상** 문서는 통과해야 한다. 여기서 붉어지면 이 축은 문서 편집에 물리는 세금이 된다.
  cat > "$DOC_DIR/ok.md" <<'DOC_OK_EOF'
- `scripts/lane-parity-rehearsal.sh` 를 돌리십시오.
- `bash scripts/lane-parity-rehearsal.sh --strict` 로도 됩니다(명령 안에 박힌 경로).
- 조사 붙임 `scripts/lane-parity-rehearsal.sh를` · 팩 상대 표기 `hooks/role-capability-gate.sh`
- 서식 인용(단일 경로 아님): `docs/*.md` · `scripts/{a,b}.sh` · `docs/<문서명>.md` · `tools/` 폴더
- 저장소 밖 · 과거 트리: `~/Desktop/CYSjavis/x/tools/zz.py` · `git show v0.14.30:src/bin/zz.rs`
DOC_OK_EOF
  LANE_PARITY_DOCS="$DOC_DIR/ok.md" existence_axes > "$SELF_TMP/doc-ok.log" 2>&1
  DOC_OK_RC=$?
  if [ $DOC_OK_RC -ne 0 ]; then
    cat "$SELF_TMP/doc-ok.log"
    echo "::error::자기 검체 실패 — 정상 문서(조사·글롭·저장소 밖 경로 혼재)에서 4단계가 exit $DOC_OK_RC 를 냈다(거짓 양성 · 정상 문서 편집이 3레인을 막는다)" >&2
    exit 1
  fi
  echo "[자기 검체] 정상 문서(조사·글롭·저장소 밖·과거 트리 혼재) → exit 0 (거짓 양성 없음)"

  # ② 음성 대조 A: 명령 **안에 박힌** 부재 경로 → exit 1 + 그 경로를 사유로 낸다.
  #    (D12 의 실제 형태가 "수집 도구: `tools/queue_remeasure.py --since …`" 였다)
  printf '%s\n' '수집 도구: `python3 tools/zz_no_such_tool.py --since <시각>`' > "$DOC_DIR/bogus.md"
  LANE_PARITY_DOCS="$DOC_DIR/bogus.md" existence_axes > "$SELF_TMP/doc-bogus.log" 2>&1
  DOC_RC=$?
  if [ $DOC_RC -ne 1 ] \
     || ! grep -qF -- 'tools/zz_no_such_tool.py' "$SELF_TMP/doc-bogus.log" \
     || ! grep -qF -- '(D12)' "$SELF_TMP/doc-bogus.log"; then
    cat "$SELF_TMP/doc-bogus.log"
    echo "::error::자기 검체 실패 — 부재 경로를 인용한 문서에서 4단계가 exit 1 + 그 경로 사유를 내지 않았다(exit $DOC_RC)" >&2
    exit 1
  fi
  echo "[자기 검체] 부재 경로 인용 문서 → exit 1 · 사유 일치 (문서 경로 축 살아 있음)"

  # ③ 음성 대조 B: 잴 경로가 **0건**인 문서 → exit 3(폐쇄). 0건 초록은 D10 이 닫은 바로 그 구멍이다.
  printf '%s\n' '이 문서는 저장소 경로를 하나도 이름 붙이지 않습니다 — `cys status --json` 뿐입니다.' \
    > "$DOC_DIR/empty.md"
  LANE_PARITY_DOCS="$DOC_DIR/empty.md" existence_axes > "$SELF_TMP/doc-empty.log" 2>&1
  DOC_ZERO_RC=$?
  if [ $DOC_ZERO_RC -ne 3 ] || ! grep -qF -- '0건' "$SELF_TMP/doc-empty.log"; then
    cat "$SELF_TMP/doc-empty.log"
    echo "::error::자기 검체 실패 — 인용 0건 문서에서 4단계가 exit 3 + '0건' 사유를 내지 않았다(측정 ≥1 폐쇄가 없다 · exit $DOC_ZERO_RC)" >&2
    exit 1
  fi
  echo "[자기 검체] 인용 0건 문서 → exit 3 · 사유 일치 (측정 ≥1 폐쇄 살아 있음)"

  echo
  echo "── 자기 검체 2: 레인 대조 게이트의 변이 대조(워크플로 사본 · LANE_GATE_ROOT) ─────────"
  # 게이트는 텍스트만 읽는다 — 워크플로 **사본**에 변이를 넣고 1단계가 추출한 **같은 게이트 원본**이
  # 붉어지는지 잰다(리포 트리 무접촉). 통과 대조(무변이 사본 rc=0)와 실패 대조 4종을 나란히 둔다:
  #   ①이름 변조(한 레인만 다른 이름 → 3레인 비대칭 · D3 의 '대조가 여전히 비대칭을 잡는가')
  #   ②`if: false`(완전 레인의 등재되지 않은 조건 · D9)
  #   ③필수 명령 소거(`cargo test --bin cysd` 스텝 이름·실행 줄 변조 · D4)
  #   ④필터 가드 삭제(`cargo_filter_count --lib readiness::` 선행 호출 제거 · D10)
  MUT_ROOT="$SELF_TMP/mut"
  mut_reset() {
    rm -rf "$MUT_ROOT"; mkdir -p "$MUT_ROOT/.github/workflows"
    for w in ci-branch release pack-release windows-build windows-health; do
      cp ".github/workflows/$w.yml" "$MUT_ROOT/.github/workflows/$w.yml"
    done
  }
  mut_expect() {  # $1=기대 rc · $2=라벨 · $3=사유 grep 패턴(고정 문자열)
    LANE_GATE_ROOT="$MUT_ROOT" python3 "$GATE_SRC" > "$SELF_TMP/mut.log" 2>&1
    local rc=$?
    if [ "$rc" -ne "$1" ]; then
      cat "$SELF_TMP/mut.log"
      echo "::error::자기 검체 2 실패 — $2: 기대 exit $1 · 실제 exit $rc" >&2
      exit 1
    fi
    if ! grep -qF -- "$3" "$SELF_TMP/mut.log"; then
      cat "$SELF_TMP/mut.log"
      echo "::error::자기 검체 2 실패 — $2: exit 는 맞지만 사유 '$3' 가 로그에 없다(다른 이유로 붉어졌다)" >&2
      exit 1
    fi
    echo "[자기 검체 2] $2 → exit $rc · 사유 일치"
  }
  mut_reset; mut_expect 0 "무변이 사본" "비대칭 0"
  mut_reset; python3 - "$MUT_ROOT/.github/workflows/pack-release.yml" <<'PYM'
import sys
p = sys.argv[1]; t = open(p, encoding="utf-8").read()
assert t.count("test_pyseal_census") >= 1, "변이 앵커 부재(test_pyseal_census)"
t = t.replace("test_pyseal_census", "test_pyseal_censux")     # 한 레인에서만 이름이 갈린다
open(p, "w", encoding="utf-8", newline="").write(t)
PYM
  mut_expect 1 "이름 변조(pack-release 만 test_pyseal_censux)" "test_pyseal_census"
  mut_reset; python3 - "$MUT_ROOT/.github/workflows/pack-release.yml" <<'PYM'
import sys
p = sys.argv[1]; t = open(p, encoding="utf-8").read()
a = "      - name: 팩 검체 — 자원 게이트·함대CPU·역할 좌석 (WP-7 R3 3레인 등재 · pack-only 서명전)\n"
assert t.count(a) == 1, "변이 앵커 부재(WP-7 pack-only 스텝)"
t = t.replace(a, a + "        if: false\n", 1)
open(p, "w", encoding="utf-8", newline="").write(t)
PYM
  mut_expect 1 "if: false(pack-release WP-7 스텝)" "if: false"
  mut_reset; python3 - "$MUT_ROOT/.github/workflows/ci-branch.yml" <<'PYM'
import sys
p = sys.argv[1]; t = open(p, encoding="utf-8").read()
run = "cargo test --bin cysd -- --test-threads=1 --skip hwmon::"
name = "- name: cargo test --bin cysd ("
assert t.count(run) == 1 and t.count(name) == 1, "변이 앵커 부재(cysd 스텝)"
t = t.replace(run, "cargo test --bin cys -- --test-threads=1 --skip hwmon::", 1)
t = t.replace(name, "- name: cargo test --bin cys (", 1)
open(p, "w", encoding="utf-8", newline="").write(t)
PYM
  mut_expect 1 "필수 명령 소거(ci-branch cysd 스텝)" "cargo test --bin cysd"
  mut_reset; python3 - "$MUT_ROOT/.github/workflows/windows-health.yml" <<'PYM'
import sys
p = sys.argv[1]; t = open(p, encoding="utf-8").read()
a = "          cargo_filter_count --lib readiness::\n"
assert t.count(a) == 1, "변이 앵커 부재(readiness 가드)"
t = t.replace(a, "", 1)
open(p, "w", encoding="utf-8", newline="").write(t)
PYM
  mut_expect 1 "필터 가드 삭제(windows-health readiness::)" "cargo_filter_count --lib readiness::"
fi
