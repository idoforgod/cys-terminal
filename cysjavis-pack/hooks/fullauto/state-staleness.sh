#!/bin/sh
# state-staleness.sh — Stop 등록용 (설계 §2 컴포넌트 3 'staleness' Phase 1 · 구현물 4b)
#
# 설치 위치: ~/.cys/local/hooks/state-staleness.sh (settings.json Stop 배열에 가산 등록)
#
# 역할: 원장의 마지막 major 이벤트(commit|task_done|handoff) ts 와 SESSION_STATE.md mtime 을
#   대조해 '주요 이벤트 이후 SESSION_STATE 미갱신'이면 원장에 staleness 레코드를 **기록만** 한다.
#   ★Phase 1 계약: 비차단·경고 주입 없음·항상 exit 0. 차단(GATE 클래스) 격상은 실측 후 별도 상신.
#   중복 억제는 javis_state_ledger.py 가 세션 마커로 처리한다(Stop 은 턴마다 발화한다).
#
# 안전: 무출력(Stop 훅 stdout 오염 금지)·모든 실패 무해 흘림·항상 exit 0.
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/../_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(state-staleness)" >&2; exit 0; }
set +e
[ -n "$CYS_STATE_LEDGER_DISABLE" ] && exit 0

IN=$(cat 2>/dev/null)

# G22: 인터프리터 해소는 프리루드 단일 소유(python3→python→py). 사본 재구현 금지(RC4).
PY="${CYS_PY:-}"
[ -n "$PY" ] || exit 0
# 스크립트 해소(팩 갱신 면역): 명시 env > 팩 bin > 사용자 로컬 bin. 셋 다 없으면 무동작.
BIN="${CYS_STATE_LEDGER_BIN:-}"
if [ -z "$BIN" ] || [ ! -f "$BIN" ]; then
  BIN="${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_state_ledger.py"
  [ -f "$BIN" ] || BIN="${CYS_LOCAL_DIR:-$HOME/.cys/local}/bin/javis_state_ledger.py"
fi
[ -f "$BIN" ] || exit 0

TO=""
if command -v timeout >/dev/null 2>&1; then TO="timeout 3"
elif command -v gtimeout >/dev/null 2>&1; then TO="gtimeout 3"; fi

SID=$(printf '%s' "$IN" | "$PY" -c 'import json,sys
try:
    d = json.load(sys.stdin)
    s = (d.get("session_id") or "nosession") if isinstance(d, dict) else "nosession"
except Exception:
    s = "nosession"
print(str(s)[:64])' 2>/dev/null)
[ -n "$SID" ] || SID="nosession"

$TO "$PY" "$BIN" staleness-check --session "$SID" >/dev/null 2>&1
exit 0
