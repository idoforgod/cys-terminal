#!/bin/sh
# agy(Antigravity CLI) 상태줄 전용 래퍼 (0.14.42) — 훅이 아니라 agy settings.json 의 statusLine 명령이다.
#   agy 는 상태가 바뀔 때마다 이 명령을 부르고 stdin 으로 상태 JSON 을 넘긴다. 쿼터 숫자만 cys 로 보낸다
#   (판별·예산·빈도 상한은 전부 `cys usage-report-stdin --agy` 안 — cys.rs run_usage_report_stdin).
#   · cys 창 좌석(CYS_SURFACE_ID 있음)에서만 보낸다. 없으면 아무것도 보내지 않는다(창 밖 agy).
#   · 명령 끝의 `--cys-autolink` 는 cys 가 자동으로 넣었다는 표지다 — 이 스크립트는 인자를 읽지 않는다.
#   · `--agy` 를 모르는 옛 cys(0.14.41 이하 — 팩만 새것으로 남은 다운그레이드)는 인자 오류로 곧바로 끝난다:
#     보내지도 출력하지도 않는다(옛 경로의 무예산 왕복·autostart 로 새지 않게 일부러 전용 플래그를 쓴다).
# ★불변: 이 명령은 **절대 agy 를 막지 않는다** — 모든 실패는 무해히 흘린다(exit 0 · stderr 버림).
#   외부 의존 없음(python 등) — cys 가 PATH 에 없으면 입력만 비우고 끝낸다.
if command -v cys >/dev/null 2>&1; then
  cys usage-report-stdin --agy 2>/dev/null
else
  cat >/dev/null 2>&1
fi
exit 0
