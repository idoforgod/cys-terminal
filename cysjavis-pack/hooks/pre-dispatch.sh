#!/bin/sh
# pre-dispatch.sh — PreToolUse 단일 디스패처 (SPEED_DESIGN_v3 D4)
#
# 목적: 현행 settings.json 은 도구 호출마다 PreToolUse 항목을 최대 4개(guard·cys-hook·
#   appbuild-gate·grill-gate) 별도로 등록해, Claude Code 가 항목당 프로세스를 각각 띄운다.
#   이 디스패처를 '합집합 matcher' 단일 항목으로 등록하면 Claude Code 는 프로세스 1개만
#   띄우고, 내부에서 각 서브훅의 원래 matcher 조건을 재현해 선택 실행한다.
#   ★안전게이트 로직은 1비트도 바꾸지 않는다 — 서브훅을 그대로 재호출할 뿐이다.
#
# 계약(SPEED_DESIGN_v3 D4 · 4항):
#   ① 서브훅별 matcher 를 디스패처 내부에서 재현(아래 dispatch 블록):
#      - guard      : Bash|Write|Edit|MultiEdit|NotebookEdit  (정확-일치 목록)
#      - cys-hook   : 전 도구(matcher "")
#      - appbuild   : Edit|Write|NotebookEdit                  (정확-일치 목록)
#      - grill      : Edit|Write|NotebookEdit                  (정확-일치 목록)
#      ※ Claude Code matcher 의미론(공식 docs): 영숫자·`_`·`-`·공백·`,`·`|` 만 포함한
#        matcher 는 정규식이 아니라 '정확-일치 목록'이다. 따라서 "Edit|Write|NotebookEdit"
#        은 tool_name "MultiEdit" 에 매칭되지 않는다(부분문자열 아님). guard 가 MultiEdit 를
#        별도 나열한 이유 = 정확-일치라 명시가 필요하기 때문. in_set() 이 이를 그대로 구현.
#   ② stdin 1회 버퍼링(IN=$(cat)) 후 각 서브훅에 printf 로 재급여 → 두 번째 서브훅부터
#      빈 stdin 으로 fail-open 되는 사고 차단.
#   ③ deny-wins: 서브훅을 등록 순서(guard→cys-hook→appbuild→grill)로 '전부' 실행하되
#      (차단 발생해도 나머지 실행 — cys-hook 사용량 기록 무손실), 최종 exit 는 어느 하나라도
#      exit 2 면 2. 차단 서브훅의 stdout(JSON)·stderr 를 최종 출력으로 채택.
#   ④ 경로 주입: guard 경로 하드코딩 금지 — 환경변수 CYS_GUARD_HOOK 로 주입(master 가
#      settings.json 배선). 미설정/실행불가 시 stderr 경고 후 guard 단계 skip(=exit 0 유지,
#      나머지 서브훅은 정상 실행). ★한계: 이 경우 autopilot 안전게이트가 빠지므로 master 는
#      반드시 CYS_GUARD_HOOK 을 배선해야 한다(회귀 테스트 CASE-G 로 명시).
#
# 서브훅 개별 실패(exit 1 등 비-2)는 Claude Code 원 동작과 동일하게 '비차단 에러'로 취급 —
# stderr 만 전달하고 최종 exit 는 (deny 없으면) 0 유지.

set -u

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(pre-dispatch)" >&2; exit 0; }
HOOK_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd) || HOOK_DIR="."

# 계약 ②: stdin 1회 버퍼링
IN=$(cat)

# ---- tool_name 추출(1스폰: jq 우선, 없으면 python3 폴백) ----
TOOL=""
if command -v jq >/dev/null 2>&1; then
  TOOL=$(printf '%s' "$IN" | jq -r '.tool_name // empty' 2>/dev/null)
fi
# G22: 경성 python3 → 프리루드 해소값(python3→python→py). 미해소면 아래 UNKNOWN superset 경로.
if [ -z "$TOOL" ] && [ -n "${CYS_PY:-}" ]; then
  TOOL=$(printf '%s' "$IN" | "$CYS_PY" -c 'import sys,json
try:
    d=json.load(sys.stdin); print(d.get("tool_name") or "")
except Exception:
    print("")' 2>/dev/null)
fi
# tool_name 미해석 = 파싱 실패 에러경로. 게이트를 놓치지 않도록 fail-safe superset(전 서브훅 실행).
UNKNOWN=0
[ -z "$TOOL" ] && UNKNOWN=1

# ---- 결과 집계용 임시공간 ----
TMPD=$(mktemp -d "${TMPDIR:-/tmp}/pre-dispatch.XXXXXX") || { printf '%s' "$IN" >/dev/null; exit 0; }
trap 'rm -rf "$TMPD"' EXIT INT TERM
DENY=0
STDOUT_TAKEN=0
: >"$TMPD/agg_err"

# in_set TOOL "set...": 정확-일치 목록 매칭. UNKNOWN(파싱실패)이면 항상 매칭(fail-safe).
in_set() {
  [ "$UNKNOWN" = "1" ] && return 0
  _needle=$1; shift
  case " $* " in
    *" $_needle "*) return 0 ;;
    *) return 1 ;;
  esac
}

# run_hook LABEL CMD [ARGS...]: 버퍼된 stdin 을 재급여해 서브훅 1개 실행, deny-wins 집계.
run_hook() {
  _label=$1; shift
  _out=$(printf '%s' "$IN" | "$@" 2>"$TMPD/e"); _rc=$?
  if [ "$_rc" -eq 2 ]; then
    DENY=1
    if [ -n "$_out" ]; then
      if [ "$STDOUT_TAKEN" -eq 0 ]; then
        printf '%s\n' "$_out" >"$TMPD/deny_out"; STDOUT_TAKEN=1
      else
        # 계약 추가항: 다중 서브훅이 동시에 stdout JSON → 가장 제한적(deny) 하나 채택,
        # 나머지 무시 사실을 stderr 에 기록.
        printf '[pre-dispatch] 추가 서브훅(%s) stdout JSON 무시 — deny 우선 채택본 유지\n' "$_label" >>"$TMPD/agg_err"
      fi
    fi
    [ -s "$TMPD/e" ] && cat "$TMPD/e" >>"$TMPD/agg_err"
  elif [ "$_rc" -ne 0 ]; then
    # 비차단 에러: Claude Code 원 동작(exit 1 = non-blocking)과 동등 — stderr 만 전달.
    { printf '[pre-dispatch] 서브훅(%s) 비차단 에러 rc=%s\n' "$_label" "$_rc"; [ -s "$TMPD/e" ] && cat "$TMPD/e"; } >>"$TMPD/agg_err"
  fi
  # exit 0: 정상 통과. cys-hook 등은 stdout 없음(있어도 allow 경로 stdout 은 채택 안 함).
}

# ================= dispatch (등록 순서 보존: guard → cys-hook → appbuild → grill) =================

# ① guard (matcher: Bash Write Edit MultiEdit NotebookEdit) — 경로 주입(CYS_GUARD_HOOK)
if in_set "$TOOL" Bash Write Edit MultiEdit NotebookEdit; then
  if [ -n "${CYS_GUARD_HOOK:-}" ] && [ -x "${CYS_GUARD_HOOK}" ]; then
    run_hook guard "$CYS_GUARD_HOOK"
  elif [ -n "${CYS_GUARD_HOOK:-}" ]; then
    printf '[pre-dispatch] CYS_GUARD_HOOK 지정됐으나 실행 불가(%s) — guard 단계 skip(exit 0 유지)\n' "$CYS_GUARD_HOOK" >>"$TMPD/agg_err"
  else
    printf '[pre-dispatch] CYS_GUARD_HOOK 미설정 — autopilot guard 단계 skip(exit 0 유지). master 배선 필요.\n' >>"$TMPD/agg_err"
  fi
fi

# ② cys-hook (matcher: 전 도구 "") — OBSERVABILITY, 무조건 실행(사용량 기록 무손실)
run_hook cys-hook sh "$HOOK_DIR/cys-hook.sh"

# ③ appbuild-gate (matcher: Edit Write NotebookEdit)
if in_set "$TOOL" Edit Write NotebookEdit; then
  run_hook appbuild sh "$HOOK_DIR/appbuild-gate.sh"
fi

# ④ grill-gate (matcher: Edit Write NotebookEdit)
if in_set "$TOOL" Edit Write NotebookEdit; then
  run_hook grill sh "$HOOK_DIR/grill-gate.sh"
fi

# ================= 최종 출력(deny-wins) =================
if [ "$DENY" -eq 1 ]; then
  [ -f "$TMPD/deny_out" ] && cat "$TMPD/deny_out"
  [ -s "$TMPD/agg_err" ] && cat "$TMPD/agg_err" >&2
  exit 2
fi
[ -s "$TMPD/agg_err" ] && cat "$TMPD/agg_err" >&2
exit 0
