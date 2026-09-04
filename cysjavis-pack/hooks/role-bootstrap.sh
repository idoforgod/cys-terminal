#!/bin/sh
# role-bootstrap.sh — UserPromptSubmit 훅 **자기완결 런처** (부트 v2 명세 §2-1 · 0.14.30 W-A A2)
#
# 이 파일의 존재 이유(명세 §0 G4 — 정직하게 축소된 목표): 감지 hot path 에서 셸 스크립트 의존을
# **이 1파일**로 줄인다. 공용 프리루드를 소스하지 않는 것은 실수가 아니라 **의도**다 — 프리루드는
# 함수 수십 종·인터프리터 해소·경로 정규화를 끌고 오고, 그 전부가 Windows 에서 하나씩 고장난
# 이력이 있다. 본체(`role-bootstrap-legacy.sh`)는 그 의존을 그대로 유지한다.
#
# 계약(종전과 동일 — 이 파일이 지켜야 하는 것):
#   · **반드시 exit 0** 으로 끝난다. 훅 실패가 사람의 프롬프트를 깨면 안 된다.
#   · stdout 은 훅 계약 JSON(`{"hookSpecificOutput":{...}}`) 1줄이거나 **아무것도 없다**.
#   · 비-cys 터미널에서는 **무발화·무부작용**(상태 디렉터리 생성도 데몬 왕복도 하지 않는다).
#   · 판정 불가(상태 디렉터리 쓰기 불가 등)는 실패가 아니라 **무발화 + 고지 1줄**이다(T1-1).
#
# ★위임 규칙(치명 — 실측으로 고친 것):
#   명세 §2-1 초안은 `cys hook user-prompt-submit --input <파일>` 의 rc 0 을 '처리완료'로 읽고
#   레거시를 건너뛴다. 그런데 **현행 CLI 에서 rc 0 은 정반대 뜻**이다 — `HOOK_EXIT_PROCEED = 0`
#   (src/bin/cys.rs) 은 '종전 게이트로 계속 진행하라'다. 그리고 현행 CLI 에는 `--input` 자체가
#   없다(실측: rc 2 + `error: unexpected argument '--input' found`). 초안대로 넣으면 W-B 의
#   `--input` 파이프라인이 착지하기 전까지 **모든 마스터 선언이 무음 사망**한다.
#   그래서 rc 를 해석하기 전에 **능력 프로브**를 먼저 돌린다: `--help` 출력에 `--input` 이
#   있어야만 신 계약(0|3 = 처리완료)을 적용하고, 없으면 위임 자체를 하지 않고 본체로 간다.
#   이 순서라야 A2(팩)와 B2(데몬/CLI)의 착지 순서가 무엇이든 안전하다.
#   ★rc 를 1차 근거로 삼지 않는 이유는 이 팩이 이미 배운 것이다 — 0 은 셸에서 가장 흔한 사고값이라
#     목(mock) `cys` 하나로 게이트가 통째로 증발한 실사고가 있었다. 능력 프로브는 **양성 증거**다.
#
# 롤백: `CYS_BOOT_GATES=0` 은 본체가 소비한다(이 런처는 새 노브를 만들지 않는다).

set +e

# ── ① 좌석 게이트(최선두) — 비-cys 터미널은 여기서 끝난다 ────────────────────────────────
# 종전 본체가 프리루드 함수로 걸던 게이트와 **같은 술어**다(surface id 자기신고 2벌).
# 이 게이트가 먼저 서야 하는 이유: 아래에서 상태 디렉터리를 만들고 파일을 쓴다 — 임의 claude
# 세션에서 그 부작용이 되살아나면 이 게이트가 애초에 도입된 사고(preflight 변형·데몬 autostart·
# boot-last 오염)가 그대로 재발한다.
[ -n "${CYS_SURFACE_ID:-}" ] || [ -n "${AITERM_SURFACE_ID:-}" ] || exit 0

# ── ② 레인 가드 — 타 레인 팩의 훅이 이 레인에서 도는 것을 막는다 ─────────────────────────
# 판정 불능은 전부 **통과**(fail-open)다 — 이 가드는 오살보다 오탐이 안전한 축이 아니다.
_cys_lane_guard() {
  [ "${CYS_HOOK_LANE_GUARD:-1}" = "0" ] && return 0
  [ -n "${CYS_PACK_DIR:-}" ] || return 0
  [ -f "${CYS_PACK_DIR}/hooks/role-bootstrap.sh" ] || return 0   # ① 레인 쪽이 진짜 팩인가
  _lg_d="${0%/*}"
  [ "$_lg_d" = "${0:-}" ] && _lg_d="."
  _lg_d="$(cd "$_lg_d" 2>/dev/null && pwd -P)" || _lg_d=""
  [ -n "$_lg_d" ] || return 0
  [ "${_lg_d##*/}" = "hooks" ] || return 0                       # ② 훅 쪽이 진짜 팩인가(대칭)
  _lg_root="${_lg_d%/*}"
  _lg_lane="$(cd "${CYS_PACK_DIR}" 2>/dev/null && pwd -P)" || _lg_lane=""
  [ -n "$_lg_lane" ] || return 0
  [ "$_lg_root" = "$_lg_lane" ] && return 0                      # ③ 같은 팩 → 통과
  echo "[cys-hook] 타 레인 팩 훅 조기 종료(hook=$_lg_root lane=$_lg_lane)" >&2
  exit 0
}
_cys_lane_guard

# ── ③ 고지 발행기 — 셸 printf 전용(외부 명령 0) ──────────────────────────────────────────
# 본체의 발행기와 **같은 형상**이어야 한다: 줄 선두가 `{"hookSpecificOutput"` 여야 소비자
# (검체·모델)가 그 줄을 집는다. printf 라 인용부호·역슬래시·개행은 실을 수 없다.
_cys_note() {
  printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"%s"}}\n' "$1"
}

# ── ④ 상태 경로 2벌(T2-1) — sh 명령용과 네이티브 인자용을 분리한다 ──────────────────────
# msys(Git Bash)에서 `cygpath -w` 결과를 sh 의 `mkdir`/`cat` 에 그대로 쓰면 다시 POSIX 로
# 되변환되는 경로를 타지만, 그 값을 **네이티브 exe 인자**로 넘길 때는 Windows 표기가 맞다.
# 그래서 디렉터리·파일 조작은 STATE(POSIX)로, `cys` 인자는 아래에서 한 번 더 변환해 넘긴다.
STATE="${CYS_STATE_DIR:-${HOME:-${USERPROFILE:-.}}/.cys/state}"
if command -v cygpath >/dev/null 2>&1; then
  STATE="$(cygpath -u "$STATE" 2>/dev/null || printf '%s' "$STATE")"
fi
if ! mkdir -p "$STATE" 2>/dev/null; then
  # T1-1: 종전 훅은 무발화 순간에도 모델에 **고지**했다. 무발화를 무음으로 만들면 사람은
  #       "부트가 왜 안 됐는지"를 볼 수 없다 — 판정 불가는 조용히 접히면 안 된다.
  cat >/dev/null 2>&1
  _cys_note "[cys-hook] 상태 디렉토리 쓰기 불가 — 부트 미발화(판정 불가)"
  exit 0
fi

# ── ⑤ 훅 입력을 파일로 받는다(본체는 stdin 대신 이 파일을 읽는다) ───────────────────────
IN="$STATE/hook-input-$$.json"
if ! cat > "$IN" 2>/dev/null; then
  rm -f "$IN" 2>/dev/null
  _cys_note "[cys-hook] 훅 입력 저장 실패 — 부트 미발화"
  exit 0
fi

# 유계 GC(T3-4 정신) — 비정상 종료로 남은 입력 파일이 무한 누적되지 않게 최신 20개만 남긴다.
# 파이프 while 을 쓰는 이유: 경로에 공백이 있어도 단어분리로 깨지지 않는다.
ls -1t "$STATE"/hook-input-*.json 2>/dev/null | tail -n +21 | while IFS= read -r _old; do
  rm -f "$_old" 2>/dev/null
done

# ── ⑥ 신 파이프라인 위임(능력 프로브 선행) ──────────────────────────────────────────────
CYS_HOOK_INPUT_DEADLINE_S="${CYS_HOOK_INPUT_DEADLINE_S:-8}"
if command -v cys >/dev/null 2>&1 \
   && cys hook user-prompt-submit --help 2>/dev/null | grep -q -- '--input'; then
  IN_ARG="$IN"
  if command -v cygpath >/dev/null 2>&1; then
    IN_ARG="$(cygpath -w "$IN" 2>/dev/null || printf '%s' "$IN")"
  fi
  RCF="$IN.rc"
  # 자식은 진입 즉시 stdio 를 끊는다 — 자식이 훅 stdout 파이프를 쥐면 사람의 프롬프트 제출이
  # 먹통이 된다(이 팩이 실제로 치른 사고 · 본체의 배경 통보기와 같은 규율).
  ( exec >/dev/null 2>&1 </dev/null
    cys hook user-prompt-submit --input "$IN_ARG"
    printf '%s' "$?" > "$RCF" ) &
  _bg=$!
  _lim=$((CYS_HOOK_INPUT_DEADLINE_S * 10))
  _i=0
  while [ "$_i" -lt "$_lim" ]; do
    [ -f "$RCF" ] && break
    kill -0 "$_bg" 2>/dev/null || break
    sleep 0.1 2>/dev/null || { sleep 1; _i=$((_i + 9)); }
    _i=$((_i + 1))
  done
  if [ -f "$RCF" ]; then
    RC="$(cat "$RCF" 2>/dev/null)"
    rm -f "$RCF" 2>/dev/null
    # 0=처리완료 · 3=억제. 이 두 경우에만 본체를 건너뛴다 — **둘 다 도는 경로는 없다**.
    case "$RC" in
      0|3) rm -f "$IN" 2>/dev/null; exit 0 ;;
    esac
  else
    # T2-2: 데드라인 초과는 **폴백이 아니라 비동기 계속**이다. 여기서 본체로 떨어지면 같은
    #       선언이 두 경로에서 처리돼 좌석 claim·인텐트 등록이 이중으로 일어난다.
    _cys_note "[cys-hook] 부트 등록이 지연되어 백그라운드로 계속합니다(진행은 boot-progress 로 보고됩니다)"
    exit 0
  fi
fi

# ── ⑦ 본체 위임(그 밖의 모든 rc · 구 CLI · cys 부재) ────────────────────────────────────
_LEGACY="${0%/*}/role-bootstrap-legacy.sh"
[ "${0%/*}" = "${0:-}" ] && _LEGACY="./role-bootstrap-legacy.sh"
if [ ! -f "$_LEGACY" ]; then
  # 명세 초안은 무조건 `exec` 이었다 — 본체가 없으면 exit 127 이 나가 '반드시 exit 0' 계약이
  # 깨진다. 부서 팩 복제 목록에 본체가 빠지면 확정적으로 재현되는 갈래라 명시 고지로 닫는다.
  rm -f "$IN" 2>/dev/null
  _cys_note "[cys-hook] 부트 본체(role-bootstrap-legacy.sh) 부재 — 부트 미발화. 팩 재설치로 복구하라"
  exit 0
fi
# ★인터프리터: 본체는 bash 를 요구한다(bash 전용 문법 2곳 · dash 에서 Bad substitution 으로
#   죽는다 — `sh -n` 문법 검사로는 잡히지 않는 런타임 사망이다). 명세의 `exec sh` 를 그대로
#   쓰면 /bin/sh 가 dash 인 배포판(대부분의 리눅스)에서 본체가 통째로 죽는다.
_CYS_SH=sh
command -v bash >/dev/null 2>&1 && _CYS_SH=bash
exec "$_CYS_SH" "$_LEGACY" "$IN"
