#!/bin/bash
# vibecoding regression 넛지 (옵트인) — PostToolUse(Bash)
# 헌법 4조(Regression 게이트): 기존 기능 자동 검증 통과 전 done 금지.
# javis_task ... set-status ... done 명령을 감지하면 테스트 스위트 존재·최근 실행
# 여부를 얕게 점검해 stderr 경고만 낸다(non-blocking). ★차단하지 않는다 — §C5에서
# done 판정은 javis_task evidence 게이트 단일 소유이며, 이 훅은 evidence 보조 넛지다.
#
# 부트체인 안전 3원칙 (이 훅은 옵트인 — 어디에도 자동 등록하지 않는다):
#   1) 5초 이내 종료 — find는 -maxdepth 2 얕은 검사만.
#   2) 어떤 실패에도 exit 0 — 세션·부트체인을 절대 차단하지 않는다.
#   3) 대상 명령 아님·cwd 부재 시 조용히 skip(에러 출력 없이 exit 0).
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/../_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(vibe-regression)" >&2; exit 0; }
# G22: 인터프리터 경성 참조 제거. 미해소면 판정 재료를 못 얻으므로 조용히 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0
INPUT=$(cat 2>/dev/null)
CMD=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('tool_input',{}).get('command',''))
except Exception: print('')" 2>/dev/null)
[ -n "$CMD" ] || exit 0

# javis_task ... set-status ... done 만 대상 (다른 상태 전이·조회는 제외)
case "$CMD" in *javis_task*) ;; *) exit 0 ;; esac
case "$CMD" in *"set-status"*) ;; *) exit 0 ;; esac
case "$CMD" in *done*) ;; *) exit 0 ;; esac

CWD=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('cwd',''))
except Exception: print('')" 2>/dev/null)
# G19: 드라이브/UNC 경로 정규화 후 절대경로 판정(Windows `C:\` cwd → $PWD 오폴백 해소)
CWD="$(cys_norm_cwd "$CWD")"
cys_is_abs "$CWD" || CWD="$PWD"
[ -d "$CWD" ] || exit 0

# 테스트 스위트 존재 얕은 탐지 (5초 예산 내 — 디렉토리 + maxdepth2 파일 마커)
HAS_TESTS=""
if [ -d "$CWD/tests" ] || [ -d "$CWD/test" ]; then HAS_TESTS=1; fi
if [ -z "$HAS_TESTS" ]; then
  if find "$CWD" -maxdepth 2 \( -name 'test_*.py' -o -name '*_test.py' \
       -o -name '*.test.ts' -o -name '*.spec.ts' -o -name '*_test.go' \) \
       2>/dev/null | head -1 | grep -q .; then HAS_TESTS=1; fi
fi

if [ -n "$HAS_TESTS" ]; then
  printf '%s\n' "[vibe-regression] set-status done 감지 — 헌법 4조: 이 작업의 회귀·테스트 스위트를 done 직전에 실행해 green을 확인했는가? evidence에 '검증 명령 → 결과'가 담겼는지 재확인하라. (옵트인 넛지·non-blocking)" 1>&2
else
  printf '%s\n' "[vibe-regression] set-status done 감지 — 테스트 스위트를 찾지 못했다. 헌법 4조(회귀 게이트): 무테스트 legacy면 §C2.3 만료형 waiver 경로가 필요하다(무근거 done 금지). (옵트인 넛지·non-blocking)" 1>&2
fi
exit 0
