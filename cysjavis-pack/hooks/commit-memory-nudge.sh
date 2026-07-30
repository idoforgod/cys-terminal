#!/bin/bash
# javis 자기교정 — PostToolUse(Bash) 시 git commit 감지 → 장기기억 증류 넛지(shadow)
# 설계: 외부 메모리 아키텍처 접목 항목④ (커밋 후 메모리 갱신 트리거)
# 역할: git commit(생성형) 직후 master에게 "이 커밋이 아키텍처 결정·중요 피드백이면
#       증류 고려하라"를 additionalContext로 리마인드. 작성은 master 판단(자동작성 0).
# 안전: graceful, 반드시 exit 0. 넛지(리마인더)만 — 강제 아님.
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(commit-memory-nudge)" >&2; exit 0; }
# G22: 인터프리터 경성 참조 제거. 미해소면 넛지 재료를 못 얻으므로 조용히 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0

INPUT=$(cat 2>/dev/null)
CMD=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('tool_input',{}).get('command',''))
except Exception: print('')" 2>/dev/null)

# git commit(생성형)만 — 'git commit ' 또는 정확히 'git commit'으로 끝나는 형태.
# 'git commit-msg'(하이픈)·log/show/status/diff 등 조회는 제외. (echo/grep 안의 문자열은
# PostToolUse라 이미 실행된 무해 명령 — shadow 넛지라 드문 오탐은 수용)
case "$CMD" in
  *"git commit "*|*"git commit") ;;
  *) exit 0 ;;
esac
case "$CMD" in
  *"--dry-run"*) exit 0 ;;
esac

MSG='방금 git commit 했다. 이 커밋이 아키텍처 결정·중요 피드백·비자명 접근법이면 장기기억 증류를 고려하라(작성은 master 판단 — 자동작성 0): '"${CYS_PY:-python3}"' "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_memory.py" add --type feedback|project --name <slug> --desc "<한 줄>" --outcome success|failure|neutral --body "<사실>". 단순 수정·리팩터·문서오타면 무시. (shadow 넛지 — 강제 아님)'

# PostToolUse additionalContext 로 master 컨텍스트에 주입 (python으로 JSON 안전 생성)
printf '%s' "$MSG" | "$CYS_PY" -c "import json,sys
print(json.dumps({'hookSpecificOutput':{'hookEventName':'PostToolUse','additionalContext':sys.stdin.read()}}, ensure_ascii=False))" 2>/dev/null
exit 0
