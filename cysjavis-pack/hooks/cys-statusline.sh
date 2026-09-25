#!/bin/sh
# Claude Code statusline 래퍼 (cys-terminal T5 Phase 2-A):
#   claude가 매 assistant 메시지마다 실행하는 statusline command. stdin으로 받은 statusline
#   JSON을 cysd에 push(usage.report)해 pane 헤더 배지에 ctx%·5h·7d rate limit을 띄우고,
#   사람용 statusline 한 줄을 stdout으로 돌려준다. surface는 CYS_SURFACE_ID(claude PTY 상속).
#   0.14.42: 같은 스크립트를 두 경로가 더 쓴다 — 판별·분기는 전부 cys CLI(run_usage_report_stdin) 안이다.
#     · cys 창 밖 Claude 세션(CYS_SURFACE_ID 없음) → 계정 전용 보고(usage.report_account · 총예산 0.4초)
#     · agy 상태줄(settings 의 statusLine 에 이 스크립트 · 페이로드 product=antigravity) → 쿼터만 좌석 보고
# ★불변: statusline 경로는 **절대 claude를 막지 않는다** — 모든 실패는 무해히 흘린다(exit 0).
IN=$(cat)
if [ -n "$CYS_PREV_STATUSLINE" ]; then
  # 기존 statusline이 있었으면 사람용 줄은 그 명령에 위임하고 배지 push만 수행(체인 보존).
  command -v cys >/dev/null 2>&1 && printf '%s' "$IN" | cys usage-report-stdin --quiet >/dev/null 2>&1
  printf '%s' "$IN" | sh -c "$CYS_PREV_STATUSLINE"
elif command -v cys >/dev/null 2>&1; then
  # push + 사람용 한 줄 출력 (cys 자기완결 — python 등 외부 의존 없음).
  printf '%s' "$IN" | cys usage-report-stdin
fi
exit 0
