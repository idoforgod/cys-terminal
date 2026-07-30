#!/bin/sh
# W0-1 Stop hook 래퍼 — 라이브 배선 승인 시 ~/.cys/pack/hooks/ 로 복사 등록.
# 격리 기간엔 이 원본만 존재(설계서 §6 규약).
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(completion-guard)" >&2; exit 0; }

PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
# 대상 존재 가드 — .py 부재 시 exec 실패로 hook이 exit 2(오류)를 내는 걸 막고 조용히 통과(H-HOOK-2).
[ -f "$PACK/bin/javis_completion_guard.py" ] || exit 0
# G22: 인터프리터 해소(python3→python→py) + cygpath 네이티브 경로. 미해소면 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0
# F9: 외곽 타임아웃 — 3중 상한 산술: 외곽 60 > guard 자체 데드라인(SELF_DEADLINE) 50 >
#     verify 개별 상한 30 (조건 23①의 '30'은 verify 개별 상한으로 supersede — 바깥으로
#     갈수록 느슨해 안쪽이 먼저 끊는다). `timeout` 부재 환경(coreutils 없는 macOS 등)은
#     기존 계약 그대로 — guard 자체 SIGALRM 데드라인이 방어한다.
if command -v timeout >/dev/null 2>&1; then
  exec timeout 60 "$CYS_PY" "$(cys_native_path "$PACK/bin/javis_completion_guard.py")"
fi
exec "$CYS_PY" "$(cys_native_path "$PACK/bin/javis_completion_guard.py")"
