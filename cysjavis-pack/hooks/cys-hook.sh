#!/bin/sh
# Claude Code/Codex 툴 이벤트 hook (cys-terminal T7 E1-④):
#   claude가 PreToolUse/PostToolUse/Stop/SubagentStop마다 실행하는 hook. stdin으로 받은 hook
#   JSON을 cysd에 push(usage.event)해 events 테이블에 툴·스킬·에이전트 호출과 exit_code를
#   적재한다(E3 스킬 TOP·반복실패 분석 토대). surface는 CYS_SURFACE_ID(에이전트 PTY 상속).
# ★불변(OBSERVABILITY hook 클래스 전용): **이 관측 hook은 절대 에이전트를 막지 않는다** —
#   PreToolUse에서 exit≠0/JSON 출력은 툴을 차단할 수 있으므로 금지. stdout 무출력·모든 실패
#   무해히 흘림·항상 exit 0(텔레메트리가 에이전트를 깨뜨려선 안 된다).
#   ※이 불변은 **관측 hook 전용** 안전규칙이지 전면 금지가 아니다. cys에는 두 hook 클래스가 있다:
#     (a) OBSERVABILITY(이 cys-hook.sh) = 절대 차단 금지·항상 exit 0.
#     (b) GATE(appbuild-gate.sh, role-capability-gate.sh) = 설계상 deny 가능(deny-by-default act tier).
#   GATE hook의 차단은 이 불변 위반이 아니라 별개 클래스다(차단이 목적).
# ★(0.14.42 R3-4 · C4) 빠른 길 — 오버레이가 하나도 없으면(= 흔한 경우) hook JSON 을 셸 변수로 받지 않고 cys 에 곧장 흘린다.
#   `$(cat)` 서브셸+cat · `printf|cys` 파이프 · `printf|sed|head` 이벤트명 추출(외부·서브셸 5 · 도구 훅 1회 약 6~14ms)을 건너뛴다.
#   동치: 빈 입력은 cys 가 스스로 무시한다(`run_usage_event_stdin` — 빈/공백 입력 = 즉시 0 · 왕복 0 · 끝 개행은 serde 가 허용).
#   오버레이 판정은 **어떤** `<이벤트>.d` 디렉터리든 있으면 종전 경로다(이벤트별 판정의 상위집합 — 오버레이를 건너뛰는 갈래 없음).
#   ★불변 유지 — stdout 무출력 · 항상 exit 0 · **stdin 은 항상 끝까지 소진된다**(종전 `$(cat)` 과 같다):
#     cys 는 정상 경로에서 입력을 끝까지 읽는다(read_to_string). cys 가 기동 중 죽거나(dyld·서명 kill·부분 업데이트) 입력을
#     읽지 않고 실패하면(구 바이너리의 인자 오류 등) rc≠0 이라 `|| cat` 이 남은 입력을 소진한다 — 큰 PostToolUse 입력에서
#     Claude Code 쪽 쓰기가 EPIPE 를 받지 않는다(검체 CH-6). 정상 경로 비용 0. cys 부재면 cat 이 소진한다.
#   ★Windows(Git Bash · cygpath 실재)는 종전 경로 그대로다(네이티브 cys.exe 에 훅 stdin 을 직접 물리는 조합은 실측 없음).
_ov=""
for _d in "${CYS_LOCAL_DIR:-$HOME/.cys/local}/hooks/"*.d; do
  [ -d "$_d" ] && { _ov=1; break; }
done
if [ -z "$_ov" ] && ! command -v cygpath >/dev/null 2>&1; then
  if command -v cys >/dev/null 2>&1; then
    cys usage-event-stdin >/dev/null 2>&1 || cat >/dev/null 2>&1
  else
    cat >/dev/null 2>&1
  fi
  exit 0
fi
IN=$(cat)
if [ -n "$IN" ] && command -v cys >/dev/null 2>&1; then
  printf '%s' "$IN" | cys usage-event-stdin >/dev/null 2>&1
fi
# ── 사용자 로컬 훅 오버레이(~/.cys/local/hooks/<이벤트>.d/*.sh) — 업데이트 불가침 확장점 ──
# 팩 파일을 직접 고치지 않고 훅을 확장하는 공식 채널. OBSERVABILITY 클래스를 상속한다:
# 후행 실행·stdout 미노출·모든 실패 무해 흘림(사용자 훅이 에이전트를 차단할 수 없다 — 안전핵).
EV=$(printf '%s' "$IN" | sed -n 's/.*"hook_event_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
LHD="${CYS_LOCAL_DIR:-$HOME/.cys/local}/hooks/${EV}.d"
if [ -n "$EV" ] && [ -d "$LHD" ]; then
  for f in "$LHD"/*.sh; do
    [ -f "$f" ] || continue
    printf '%s' "$IN" | sh "$f" >/dev/null 2>&1 || true
  done
fi
exit 0
