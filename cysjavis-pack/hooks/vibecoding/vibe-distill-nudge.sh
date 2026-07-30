#!/bin/bash
# vibecoding distill 넛지 (옵트인) — PostToolUse(Bash)
# 헌법 10조(실수→규칙 증류): 에러 수정·리뷰 수반 작업 종결마다 재발 방지 규칙 증류 의무.
# 에러 수정형 git commit(또는 리뷰 종결 패턴)을 감지하면 §C8 증류 lifecycle 진입을
# additionalContext로 넛지한다(non-blocking·자동 증류 0 — 작성은 master 판단).
#
# 부트체인 안전 3원칙 (이 훅은 옵트인 — 어디에도 자동 등록하지 않는다):
#   1) 5초 이내 종료 — 문자열 패턴 매칭만.
#   2) 어떤 실패에도 exit 0 — 세션·부트체인을 절대 차단하지 않는다.
#   3) 의존 도구(javis_distill.py) 부재 시 조용히 skip — 없는 도구를 권하지 않는다.
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/../_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(vibe-distill-nudge)" >&2; exit 0; }
PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
[ -f "$PACK/bin/javis_distill.py" ] || exit 0   # 도구 부재 시 넛지 자체를 skip

# G22: 인터프리터 경성 참조 제거. 미해소면 넛지 재료를 못 얻으므로 조용히 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0
INPUT=$(cat 2>/dev/null)
CMD=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('tool_input',{}).get('command',''))
except Exception: print('')" 2>/dev/null)
[ -n "$CMD" ] || exit 0

# git commit(생성형)만 — 조회성 제외, dry-run 제외 (commit-memory-nudge와 동일 규약)
case "$CMD" in *"git commit "*|*"git commit") ;; *) exit 0 ;; esac
case "$CMD" in *"--dry-run"*) exit 0 ;; esac

# 에러 수정·리뷰 종결 패턴만 (단순 리팩터·기능추가·문서는 제외 — 10조 의무 범위)
case "$CMD" in
  *fix*|*Fix*|*FIX*|*bug*|*Bug*|*hotfix*|*revert*|*Revert*|*regression*|*수정*|*버그*|*에러*|*오류*|*고침*|*review*|*Review*|*리뷰*) ;;
  *) exit 0 ;;
esac

MSG='방금 에러 수정·리뷰 종결로 보이는 커밋을 했다. 헌법 10조(실수→규칙 증류): 확인된 근본원인이 있으면 §C8 증류 lifecycle에 candidate로 등재하라(자동 증류 0 — 작성은 master 판단): '"${CYS_PY:-python3}"' "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_distill.py" propose --project <루트> --body "<규칙 2줄>" --root-cause "<확인된 근본원인>" --regression-test-ref "<재발 방지 테스트>". 오탐(단순 리팩터·문서·기능추가)이면 무시. (옵트인 넛지·non-blocking)'

printf '%s' "$MSG" | "$CYS_PY" -c "import json,sys
print(json.dumps({'hookSpecificOutput':{'hookEventName':'PostToolUse','additionalContext':sys.stdin.read()}}, ensure_ascii=False))" 2>/dev/null
exit 0
