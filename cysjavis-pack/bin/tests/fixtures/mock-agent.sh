#!/bin/sh
# mock-agent.sh — 편성/동등성 테스트용 가짜 CLI (실 claude/agy/codex 대역).
# CI 에는 실 CLI 가 없으므로(설계 §5·W4) agents.json 어댑터에 이 스크립트를 주입해
# 편성·디렉티브 주입 로직만 검증한다. 동작: 각성 마커 1줄을 남기고(로스터·주입 판정용)
# stdin 이 닫힐 때까지 살아 대기한다(실 에이전트가 프롬프트를 기다리는 상태 모사).
#
# env:
#   CYS_ROLE       — 이 pane 의 역할(로스터 판정 대상)
#   CYS_SURFACE_ID — surface 식별
#   MOCK_AGENT_LOG — 각성 마커를 append 할 파일(테스트가 로스터·주입 대조에 소비). 미설정 시 무기록.
# args: 임의(예: --dangerously-skip-permissions) — 무시하고 대기만 한다.

if [ -n "${MOCK_AGENT_LOG:-}" ]; then
  # 주입된 디렉티브 해시 대조(설계 R2-4: 노드 수가 아니라 지침 온전함)를 위해
  # CYS_INJECT_DIRECTIVE(테스트가 넘긴 주입 디렉티브 경로)의 sha256 도 함께 기록.
  _dhash=""
  if [ -n "${CYS_INJECT_DIRECTIVE:-}" ] && [ -f "${CYS_INJECT_DIRECTIVE}" ]; then
    _dhash="$(shasum -a 256 "${CYS_INJECT_DIRECTIVE}" 2>/dev/null | awk '{print $1}')"
    [ -z "$_dhash" ] && _dhash="$(sha256sum "${CYS_INJECT_DIRECTIVE}" 2>/dev/null | awk '{print $1}')"
  fi
  printf 'AWAKE role=%s surface=%s directive_sha=%s\n' \
    "${CYS_ROLE:-none}" "${CYS_SURFACE_ID:-none}" "${_dhash:-none}" >> "${MOCK_AGENT_LOG}"
fi

# 살아 대기 — stdin 종료 또는 SIGTERM 까지. (테스트는 pid 를 kill 해 정리.)
if [ -n "${MOCK_AGENT_ONESHOT:-}" ]; then
  exit 0
fi
cat >/dev/null 2>&1 || true
# cat 이 즉시 EOF 를 받아도(비대화 파이프) 편성 검증은 마커로 끝났으므로 안전.
exit 0
