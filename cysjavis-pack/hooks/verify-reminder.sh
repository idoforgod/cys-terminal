#!/bin/sh
# W0-2 PostToolUse(Task|Agent) hook 래퍼 — 라이브 배선 승인 시 ~/.cys/pack/hooks/ 로 복사 등록.
# 1차 라이브 대상은 master 프로필(위임 수신자) — 등록은 승인 후 master 집행(설계서 §3.2).
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(verify-reminder)" >&2; exit 0; }

PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
# 대상 존재 가드 — .py 부재 시 exec 실패로 hook이 exit 2(오류)를 내는 걸 막고 조용히 통과(H-HOOK-2).
[ -f "$PACK/bin/javis_verify_reminder.py" ] || exit 0
# G22: 인터프리터 해소(python3→python→py) + cygpath 네이티브 경로. 미해소면 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0
exec "$CYS_PY" "$(cys_native_path "$PACK/bin/javis_verify_reminder.py")"
