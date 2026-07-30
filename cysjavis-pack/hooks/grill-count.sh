#!/bin/sh
# PostToolUse(matcher AskUserQuestion) — grill-me 결정축 카운트(evaluator).
# ★역할 분리(eval-driven 무결성): 질문하는 LLM(producer)과 distinct를 세는 이 hook
#   (evaluator)과 차단하는 grill-gate.sh(gatekeeper)를 분리한다. begin/check/end만
#   배선하고 이 count를 빠뜨리면 distinct가 영원히 0 → fail-CLOSED 마비가 된다.
# ★이 hook은 차단 게이트가 아니다 — grill_gate.py count는 마커에 누적만 하고 절대
#   exit≠0을 내지 않는다(관측·누적 성격). 따라서 항상 exit 0(에이전트 비차단).
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(grill-count)" >&2; exit 0; }
# G22: 인터프리터 경성 게이트 제거 — 프리루드 해소값 사용. 미해소면 fail-open(누적 불가).
[ -n "${CYS_PY:-}" ] || exit 0
HOOK_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd) || exit 0
GATE_PY="$HOOK_DIR/../bin/grill_gate.py"
[ -f "$GATE_PY" ] || exit 0
GATE_PY_N="$(cys_native_path "$GATE_PY")"
"$CYS_PY" "$GATE_PY_N" count >/dev/null 2>&1   # stdin(hook JSON) → count, 결과·실패 무시
exit 0
