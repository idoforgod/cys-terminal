#!/bin/bash
# vibecoding doc-sync 넛지 (옵트인) — PostToolUse(Edit|Write)
# 헌법 8조(doc-sync): 행동·구조·계약·상태 변경은 관련 문서 동반 갱신 없이 미완.
# 코드 파일을 편집했는데 working tree에 문서(.md) 변경이 하나도 없으면 stderr 경고만
# 낸다(non-blocking). ★차단(exit 2 deny)은 §C6 pilot 통과 후로 보류 — 지금은 경고뿐.
#
# 부트체인 안전 3원칙 (이 훅은 옵트인 — 어디에도 자동 등록하지 않는다):
#   1) 5초 이내 종료 — 무거운 스캔·네트워크 금지.
#   2) 어떤 실패에도 exit 0 — 세션·부트체인을 절대 차단하지 않는다.
#   3) 의존 도구(git)·경로 부재 시 조용히 skip(에러 출력 없이 exit 0).
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/../_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(vibe-doc-sync)" >&2; exit 0; }
# G22: 인터프리터 경성 참조 제거. 미해소면 판정 재료를 못 얻으므로 조용히 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0
INPUT=$(cat 2>/dev/null)

FP=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('tool_input',{}).get('file_path',''))
except Exception: print('')" 2>/dev/null)
[ -n "$FP" ] || exit 0

# 편집 대상이 코드 파일일 때만 — 문서(.md/.rst/.txt) 편집은 doc-sync 대상 아님
case "$FP" in
  *.py|*.sh|*.rs|*.ts|*.tsx|*.js|*.jsx|*.go|*.java|*.rb|*.c|*.h|*.cpp|*.cc|*.swift|*.kt) ;;
  *) exit 0 ;;
esac

# git 저장소에서만 동작(작업 트리 문서 변경 여부 판별) — git 부재 시 skip
command -v git >/dev/null 2>&1 || exit 0
DIR=$(dirname "$FP")
[ -d "$DIR" ] || exit 0
ROOT=$(git -C "$DIR" rev-parse --show-toplevel 2>/dev/null) || exit 0

# 작업 트리 변경 목록 — 코드 변경은 있는데 .md 변경이 0이면 doc-sync 리마인드
CHANGED=$(git -C "$ROOT" status --porcelain 2>/dev/null)
[ -n "$CHANGED" ] || exit 0
printf '%s\n' "$CHANGED" | grep -qiE '\.md( ->|$)' && exit 0   # 문서 변경 동반 → 통과

printf '%s\n' "[vibe-doc-sync] 코드 변경이 감지됐으나 작업 트리에 문서(.md) 갱신이 없다 — 헌법 8조(doc-sync): 행동·구조·계약·상태 변경은 관련 문서 동반 갱신 없이 미완. 문서 갱신이 불필요한 변경이면 무시하라. (옵트인 넛지·non-blocking·차단은 pilot 후)" 1>&2
exit 0
