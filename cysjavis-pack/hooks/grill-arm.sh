#!/bin/sh
# PreToolUse ARM hook (matcher Skill): grill-me 스킬 발동 시 grill_gate 자동 무장(begin).
# 의도(2026-07-16): floor(20·복잡30) 게이트의 무장이 LLM 자발 `begin` 실행에 달려
# 있던 갭을 결정론으로 봉인 — grill-me를 쓸 때마다 최소 질문 수 강제가 반드시 발동한다.
#
# ★관측/무장 클래스 — 절대 차단하지 않는다(모든 경로 exit 0 · fail-open):
#   - tool_input.skill == "grill-me" 일 때만 동작
#   - 이미 수집 중(collecting·미만료) 마커가 있으면 begin 재실행 금지(axes 리셋 방지)
#   - CYS_SURFACE_ID 부재 시 begin 자체가 미발동(엔진 fail-open 설계 그대로)
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(grill-arm)" >&2; exit 0; }
# G22: `command -v python3` 경성 게이트 → 프리루드 해소값(python3→python→py). 미해소면 fail-open.
[ -n "${CYS_PY:-}" ] || exit 0
HOOK_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd) || exit 0
GATE_PY="$HOOK_DIR/../bin/grill_gate.py"
[ -f "$GATE_PY" ] || exit 0
GATE_PY_N="$(cys_native_path "$GATE_PY")"   # Windows 네이티브 python은 POSIX 경로를 못 연다

INPUT=$(cat 2>/dev/null) || INPUT=""
[ -n "$INPUT" ] || exit 0

SKILL=$(printf '%s' "$INPUT" | "$CYS_PY" -c '
import json, sys
try:
    d = json.load(sys.stdin)
    ti = d.get("tool_input") or {}
    print(ti.get("skill") or "" if isinstance(ti, dict) else "")
except Exception:
    pass
' 2>/dev/null)
[ "$SKILL" = "grill-me" ] || exit 0

# 이미 수집 중(미만료)이면 재무장 금지 — begin은 마커를 덮어써 진행(axes)을 리셋한다.
ST=$("$CYS_PY" "$GATE_PY_N" status 2>/dev/null)
case "$ST" in
  *'"status": "collecting"'*)
    case "$ST" in
      *'"expired": false'*) exit 0 ;;
    esac ;;
esac

REQ=$(printf '%s' "$INPUT" | "$CYS_PY" -c '
import json, sys
try:
    d = json.load(sys.stdin)
    ti = d.get("tool_input") or {}
    a = ti.get("args") if isinstance(ti, dict) else ""
    print((a or "grill-me skill invocation")[:500])
except Exception:
    print("grill-me skill invocation")
' 2>/dev/null)
[ -n "$REQ" ] || REQ="grill-me skill invocation"

"$CYS_PY" "$GATE_PY_N" begin --request "$REQ" >/dev/null 2>&1
exit 0
