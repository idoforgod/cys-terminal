#!/bin/bash
# javis 자기교정 — Stop·SessionEnd 시 ①반복신호 light scan + ③메모리 정합 verify
# 설계: 외부 메모리 아키텍처 접목 항목①③ · RSI_PROTOCOL.md ① Capture
# 역할: ① transcript의 사람 교정신호를 훑어 임계 도달 시 RSI_LEDGER.md에 SHADOW 후보 적재
#       ③ 종료 시 장기기억 색인↔파일 정합을 verify, 깨졌으면 .state_log에 경고만
# 안전: graceful, 반드시 exit 0 (hook 실패가 세션을 깨지 않게). 자동적용·자동수정 0(shadow).
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(reflect-scan)" >&2; exit 0; }

INPUT=$(cat 2>/dev/null)
PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
ROOT="${CYS_ROOT:-$HOME}"

# G22: 인터프리터 경성 참조(python3) 제거 — 프리루드가 python3→python→py로 해소.
# 미해소(빈 값)면 이 훅은 판정 재료를 얻을 수 없으므로 조용히 통과한다(기존 graceful 계약 유지).
[ -n "$CYS_PY" ] || exit 0

TRANSCRIPT=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('transcript_path',''))
except Exception: print('')" 2>/dev/null)
CWD=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('cwd',''))
except Exception: print('')" 2>/dev/null)
# G19: 드라이브/UNC 경로 정규화 후 절대경로 판정(Windows `C:\` cwd 공란화 해소·POSIX 무변경)
CWD="$(cys_norm_cwd "$CWD")"
cys_is_abs "$CWD" || CWD=""   # 절대경로만 상향탐색 (무한루프 방지)

# _round 상향탐색 (save-state.sh와 동일 규약) → fallback ACTIVE_PROJECT
DIR="$CWD"; RD=""; PREV=""
while [ -n "$DIR" ] && [ "$DIR" != "/" ] && [ "$DIR" != "$PREV" ]; do
  if [ -d "$DIR/_round" ]; then RD="$DIR/_round"; break; fi
  PREV="$DIR"
  DIR=$(dirname "$DIR")
done
if [ -z "$RD" ] && [ -f "$ROOT/_round/ACTIVE_PROJECT" ]; then
  AP=$(head -1 "$ROOT/_round/ACTIVE_PROJECT" 2>/dev/null)
  [ -n "$AP" ] && [ -d "$AP/_round" ] && RD="$AP/_round"
fi
[ -z "$RD" ] && exit 0

NOW=$(date -Iseconds 2>/dev/null || date)

# ① 반복신호 light scan → RSI_LEDGER.md SHADOW 후보 (임계3·멱등·자동적용0)
if [ -n "$TRANSCRIPT" ] && [ -f "$TRANSCRIPT" ] && [ -f "$PACK/bin/javis_reflect.py" ]; then
  "$CYS_PY" "$(cys_native_path "$PACK/bin/javis_reflect.py")" scan --transcript "$TRANSCRIPT" \
    --ledger "$RD/RSI_LEDGER.md" >/dev/null 2>&1
fi

# ③ 종료 시 장기기억 정합 verify — 깨졌으면 .state_log에 경고만(자동수정 0)
if [ -f "$PACK/bin/javis_memory.py" ]; then
  if ! "$CYS_PY" "$(cys_native_path "$PACK/bin/javis_memory.py")" verify >/dev/null 2>&1; then
    echo "$NOW	WARN:memory	장기기억 색인↔파일 정합 깨짐 — javis_memory.py verify 확인 필요" \
      >> "$RD/.state_log" 2>/dev/null
  fi
fi
exit 0
