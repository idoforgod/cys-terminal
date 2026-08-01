#!/bin/sh
# Stop GATE hook: grill 수집 중(collecting)·floor 미충족이면 턴 종료를 차단한다.
# 의도(2026-07-16): 파일 쓰기가 없는 순수 계획·합의 flow에서는 기존 PreToolUse
# 게이트(Edit/Write)가 발동할 트리거 자체가 없어 조기 합의 선언을 아무도 막지 못했다.
# 이 hook이 그 갭을 닫는다 — 미해소 결정축이 floor에 미달하면 종료 대신 계속 질문하게 한다.
#
# ★grill-gate.sh와 동일 계열 GATE hook: 차단은 grill_gate.py check가 exit 2를 '명시'할
#   때만 — 마커 부재·TTL 만료·passed/done/abandoned·python 부재·크래시 전부 fail-OPEN.
#   마커는 surface별 격리라 grill 중인 pane 밖의 세션은 영향이 없다.

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(grill-stop)" >&2; exit 0; }
if [ "${1:-}" = "--self-test" ]; then
  HOOK_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd)
  [ -n "${CYS_PY:-}" ] || exit 0
  exec "$CYS_PY" "$(cys_native_path "$HOOK_DIR/../bin/grill_gate.py")" --self-test
fi

[ -n "${CYS_PY:-}" ] || exit 0        # G22 fail-open: python(3) 해소 불가
HOOK_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd) || exit 0
GATE_PY="$HOOK_DIR/../bin/grill_gate.py"
[ -f "$GATE_PY" ] || exit 0
GATE_PY_N="$(cys_native_path "$GATE_PY")"

cat >/dev/null 2>&1   # Stop hook stdin(JSON)은 check가 쓰지 않음 — 소비만

"$CYS_PY" "$GATE_PY_N" check
rc=$?
# 오직 명시적 floor 미충족(2)만 차단(check가 사유를 stderr로 출력). 그 외 전부 통과.
[ "$rc" = "2" ] && exit 2
exit 0
