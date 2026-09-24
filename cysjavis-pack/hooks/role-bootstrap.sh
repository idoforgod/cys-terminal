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
#   ★rc 계약(master 판정 2026-09-04 ①): 신 `cys hook --input`(W-B B2)은 **처리완료를 exit 6** 으로
#   낸다. rc 0 은 종전 `HOOK_EXIT_PROCEED` 의미(=본체로 계속 진행) 그대로다. 따라서 이 런처가
#   본체를 건너뛰는 rc 는 **6(처리완료)과 3(억제) 둘뿐**이고 나머지 전부(0 포함)는 본체로 간다.
#   0 을 처리완료로 읽는 분기는 존재하지 않는다 — 0 은 셸에서 가장 흔한 사고값이라 목(mock) `cys`
#   하나로 게이트가 통째로 증발한 실사고(A3=B7)가 있었다.
#   능력 프로브는 그 위에 남긴 **양성 증거** 한 겹이다: `--help` 에 `--input` 이 없으면(구 CLI)
#   위임을 시도조차 하지 않는다 — 인자 오류 rc 2 와 loud 폴백 로그를 매 선언마다 만들지 않는다.
#
# 롤백: `CYS_BOOT_GATES=0` 은 본체가 소비한다(이 런처는 새 노브를 만들지 않는다).

set +e

# ── ① 좌석 게이트(최선두) — 비-cys 터미널은 여기서 끝난다 ────────────────────────────────
# 종전 본체가 프리루드 함수로 걸던 게이트와 **같은 술어**다(surface id 자기신고 2벌).
# 이 게이트가 먼저 서야 하는 이유: 아래에서 상태 디렉터리를 만들고 파일을 쓴다 — 임의 claude
# 세션에서 그 부작용이 되살아나면 이 게이트가 애초에 도입된 사고(preflight 변형·데몬 autostart·
# boot-last 오염)가 그대로 재발한다.
[ -n "${CYS_SURFACE_ID:-}" ] || [ -n "${AITERM_SURFACE_ID:-}" ] || exit 0

# ── ①-b 공용 프리루드 — **있으면 소비, 없으면 자기완결로 강등**(master 판정 2026-09-04 ②) ──
# 자기완결의 뜻은 "프리루드가 없어도 돈다"이지 "있어도 안 쓴다"가 아니다. 프리루드는 이 런처가
# 스스로 만들 수 없는 것을 준다 — 로케일 고정 · 바이트코드 봉인(SEAL-1) · 상태 경로 네이티브 표기
# 정규화(2026-08-10 Windows 실기 근본수정) · 레인 가드. 그래서 2단으로 찾아보고 없으면 **조용히
# 강등**한다(본체와 달리 loud-skip 으로 `exit 0` 하지 않는다 — 여기서 죽으면 부트가 죽는다).
# ★`.`(dot) 는 POSIX **특수 내장**이라 대상 파일을 못 읽으면 비대화형 셸이 그 자리에서 **종료**한다
#   — `|| :` 로도 못 막는다(dash 실측: 프리루드 부재 시 런처가 rc 1 로 즉사했다). 그래서 읽기
#   가능 여부를 **먼저 검사**하고, 있을 때만 소스한다. 이 순서가 곧 '자기완결'의 실체다.
_CYS_PRELUDE="${0%/*}/_lib.sh"
[ -r "$_CYS_PRELUDE" ] || _CYS_PRELUDE="${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh"
if [ -r "$_CYS_PRELUDE" ]; then
  . "$_CYS_PRELUDE" 2>/dev/null || :
fi
command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect "$@"

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
# 프리루드가 있었다면 그쪽 레인 가드가 **source 시점에 이미 실행**됐다 — 중복 판정하지 않는다.
command -v cys_lane_guard >/dev/null 2>&1 || _cys_lane_guard

# ── ③ 고지 발행기 — 셸 printf 전용(외부 명령 0) ──────────────────────────────────────────
# 본체의 발행기와 **같은 형상**이어야 한다: 줄 선두가 `{"hookSpecificOutput"` 여야 소비자
# (검체·모델)가 그 줄을 집는다. printf 라 인용부호·역슬래시·개행은 실을 수 없다.
# ★(0.14.42 P4) 대화 승인 발급 고지(`_CYS_TT_NOTE` · ⑤-b)가 대기 중이면 **같은 줄에 합친다** —
#   발급 고지가 생겨도 stdout 계약(JSON 1줄 또는 무출력)은 그대로다. 대기 고지가 없으면 출력은
#   종전과 바이트 동일하다. 발급 고지 본문은 발급기가 인용부호·역슬래시·제어문자를 이미 걷어 냈다.
_CYS_TT_NOTE=""
_cys_note() {
  _cn_msg="${1:-}"
  if [ -n "$_CYS_TT_NOTE" ]; then
    if [ -n "$_cn_msg" ]; then _cn_msg="$_cn_msg / $_CYS_TT_NOTE"; else _cn_msg="$_CYS_TT_NOTE"; fi
    _CYS_TT_NOTE=""
  fi
  printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"%s"}}\n' "$_cn_msg"
}
# 대기 중인 발급 고지만 단독으로 낸다 — 런처가 자기 고지 없이 떠나는 갈래(처리완료·억제의 EXIT 트랩 ·
# 본체 exec 직전).
_cys_tt_flush() {
  [ -n "$_CYS_TT_NOTE" ] || return 0
  _cys_note ""
}

# ── ③-b 경로 헬퍼 — 디렉터리 부분(구분자 정규화 선행) ─────────────────────────────────────
# 정의만 여기로 올렸다(⑤-b 발급기 해소가 ⑥ 위임보다 **앞**에서 쓴다). 왜 백슬래시를 먼저 바꾸는지와
# 해소 순서는 ⑦ 의 주석이 정본이다(IG-11 2차 · Windows 백슬래시 절대경로).
_cys_dirpart() {
  [ -n "${1:-}" ] || return 1
  _cdp=$(printf '%s' "$1" | tr '\\' '/')
  case "$_cdp" in
    */*) printf '%s' "${_cdp%/*}" ;;
    *)   return 1 ;;
  esac
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

# ── ⑤-b 대화 승인 1회용 팀 생성 토큰 발급(0.14.42 · 설계 §6-4·§6-6·§7-3·§11 R11·§13 P4) ────────
# 오너가 master 의 질문("이 내용으로 만들까요?")에 이 좌석에서 직접 "만들어"라고 치면, 그 프롬프트에서
# 1회용 토큰을 발급한다. 판정·발급은 `bin/javis_teamtoken.py issue` 가 단일 소유하고(배달 원장 대조로
# 오너 실키 입력을 가린다 — 기계 배달엔 발급하지 않는다), 인터프리터가 필요한 호출은 프리루드 소비
# 파일 `teamtoken-issue.sh` 가 한다(이 런처는 프리루드 규약 심볼을 쓰지 않는다 — 자기완결 계약).
# ★자리가 여기인 이유: 아래 ⑥ 은 기계 유래 프롬프트를 처리완료(rc 6)로 닫아 본체를 건너뛴다. 발급
#   호출이 본체에 있으면 바로 그 경로(기계 배달된 "그래 만들어")에서 거부 사유가 원장에 남지 않는다.
#   모든 프롬프트가 지나는 한 점은 여기다.
# ★비용 게이트(매 프롬프트 공통 — 인터프리터를 띄우기 **전**에 거른다):
#   ⓐ 토큰 원장(`teamtoken-<레인>.jsonl`)이 하나도 없다 = 질문이 열린 적이 없다 → 셸 글롭 1회로 끝
#      (외부 명령 0). 레인 접미 규약을 여기 복사하지 않는다 — 전 레인 글롭은 상위집합이라 안전하다.
#   ⓑ 있어도 최근 `CYS_TT_WINDOW_S` 초 동안 안 움직였다 = 쓸 수 있는 질문이 없다 → date·stat 각 1회로 끝.
#      질문은 열릴 때 원장에 쓰이고 수명이 300초라, 열린 질문이 살아 있으면 원장은 300초 안에 움직였다.
#      창을 두 배(600초)로 잡은 것은 막 만료된 질문에 "확인 시간이 지나 다시 여쭙습니다"(§10)를 한 번
#      말해 줄 여유다. 그보다 오래된 만료 질문은 master 의 `javis_teamtoken.py status` 가 보여 준다(§7-3).
#   ★게이트의 실패 방향은 '발급기를 부른다'다(시각·mtime 판독 불가 = 최근으로 본다). 거르는 쪽 오판은
#     발급 0(= fail-closed)일 뿐 허용이 되지 못한다 — 판정은 언제나 발급기가 원장 내용으로 한다.
# ★실패 정책(이 런처와 같다): 발급기가 없거나 죽거나 늦어도 프롬프트 제출은 막지 않는다 — 발급기는
#   전경에서 돌되 stdio 를 이 훅에서 끊고(`</dev/null >/dev/null` · stderr 는 진단용으로 통과) 자기
#   데드라인(인터프리터 호출 6s + 고지 3s)으로 유계다. 결과는 고지 파일 1줄로만 돌아온다(파이프 점유 0).
CYS_TT_WINDOW_S=600
_cys_tt_recent() {
  _tt_now=""
  for _tt_f in "$STATE"/teamtoken-*.jsonl; do
    [ -f "$_tt_f" ] || continue
    if [ -z "$_tt_now" ]; then
      _tt_now="$(date +%s 2>/dev/null)"
      case "$_tt_now" in ''|*[!0-9]*) return 0 ;; esac
    fi
    _tt_m="$(stat -c %Y "$_tt_f" 2>/dev/null)"
    case "$_tt_m" in ''|*[!0-9]*) _tt_m="$(stat -f %m "$_tt_f" 2>/dev/null)" ;; esac
    case "$_tt_m" in ''|*[!0-9]*) return 0 ;; esac
    [ $((_tt_now - _tt_m)) -lt "$CYS_TT_WINDOW_S" ] && return 0
  done
  return 1
}
if _cys_tt_recent; then
  _TTH=""
  for _tt_d in "$(_cys_dirpart "${BASH_SOURCE:-}")" "$(_cys_dirpart "${0:-}")" \
               "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks"; do
    if [ -n "$_tt_d" ] && [ -f "$_tt_d/teamtoken-issue.sh" ]; then
      _TTH="$_tt_d/teamtoken-issue.sh"
      break
    fi
  done
  if [ -n "$_TTH" ]; then
    _TTN="$STATE/teamtoken-note-$$.txt"
    rm -f "$_TTN" 2>/dev/null
    sh "$_TTH" "$IN" "$_TTN" </dev/null >/dev/null
    _CYS_TT_KIND=""
    if [ -s "$_TTN" ]; then
      { IFS= read -r _CYS_TT_KIND; IFS= read -r _CYS_TT_NOTE; } < "$_TTN" || :
    fi
    rm -f "$_TTN" 2>/dev/null
    # 고지가 대기 중이면 이 런처의 **모든 exit** 가 그것을 내게 한다(처리완료·억제 갈래는 자기 고지가
    # 없다). 자기 고지가 있는 갈래는 `_cys_note` 가 이미 합쳐 비웠으므로 트랩은 아무것도 안 한다.
    # exec(본체 위임)는 EXIT 트랩을 태우지 않는다 — 그 갈래는 ⑦ 이 exec 직전에 명시로 낸다.
    [ -n "$_CYS_TT_NOTE" ] && trap '_cys_tt_flush' EXIT
  else
    echo "[cys-hook] role-bootstrap: 대화 승인 발급기(hooks/teamtoken-issue.sh) 부재 — 이 발화는 승인 판정 없이 지나간다(발급 0 · 팩 재설치로 복구)" >&2
  fi
fi

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
    # 6=처리완료 · 3=억제. **이 둘만** 본체를 건너뛴다(0 은 proceed 라 본체로 간다).
    # (⑤-b 의 발급 고지는 이 갈래에서 EXIT 트랩이 단독으로 낸다 — 이 줄은 음성 대조 MUT-1 의 앵커다.)
    case "$RC" in
      6|3) rm -f "$IN" 2>/dev/null; exit 0 ;;
    esac
  else
    # T2-2: 데드라인 초과는 **폴백이 아니라 비동기 계속**이다. 여기서 본체로 떨어지면 같은
    #       선언이 두 경로에서 처리돼 좌석 claim·인텐트 등록이 이중으로 일어난다.
    _cys_note "[cys-hook] 부트 등록이 지연되어 백그라운드로 계속합니다(진행은 boot-progress 로 보고됩니다)"
    exit 0
  fi
fi

# ── ⑦ 본체 위임(그 밖의 모든 rc · 구 CLI · cys 부재) ────────────────────────────────────
# ★본체 경로는 **CWD 에 의존하지 않는다**(A2 회귀 · windows-health 가 적발).
#   종전: `_LEGACY="${0%/*}/…"` + `$0` 에 슬래시가 없으면 `./role-bootstrap-legacy.sh`.
#   그런데 `sh role-bootstrap.sh`(인터프리터 + 이름만 · PATH 해소)로 부르면 argv0 이
#   이름뿐이라 `${0%/*}` 가 `$0` 를 그대로 돌려주고, 폴백이 **CWD 상대**가 된다.
#   그러면 본체가 **실재하는데도** '부재 — 부트 미발화'로 판정된다(무음이 아니라 **거짓 고지**라
#   더 나쁘다: 팩 재설치를 처방하지만 팩은 멀쩡하다). 실측 적발: H-WIN-11·H-MISSION-1·H-DETECT-10.
#   해소 순서 — ①BASH_SOURCE 디렉터리(argv0 이 이름뿐이어도 정확하다 · 실측 확인)
#              ②$0 디렉터리(슬래시가 있을 때만) ③팩 계약 경로(프리루드와 동형 2단 폴백)
#              ④그래도 없으면 **정직 실패**(CWD 상대 추정 금지 — 무음 통과보다 정직한 고지).
# ★구분자 정규화가 **먼저**다(IG-11 2차 · 2026-09-04 Windows 실기 적발).
#   `${p%/*}` 는 **슬래시에서만** 자른다. Git Bash 가 넘기는 백슬래시 절대경로
#   (`C:\Users\…\hooks\role-bootstrap.sh`)에는 `/` 가 하나도 없어서 `%/*` 가 **원문을
#   그대로** 돌려주고, "슬래시가 없다"는 판정과 구별되지 않아 두 후보가 **동시에 빈손**이 된다.
#   그러면 팩 폴백(③)만 남는데 격리 하네스의 가짜 팩에는 본체가 없어 '부재'로 정직 실패한다 —
#   1차 수리(CWD 제거)가 결함을 한 층 위로 옮겼을 뿐이었다. macOS 는 ①이 성공해 로컬만 초록이었다
#   (로컬↔CI 갈림의 정체가 이것이다).
#   Git Bash 는 슬래시 경로를 그대로 받으므로 **백슬래시를 슬래시로 바꾼 뒤** 자른다.
#   드라이브 문자(`C:`)는 건드리지 않는다 — 정규화는 구분자에만 적용된다.
#   (그 함수 `_cys_dirpart` 의 정의는 ③-b 에 있다 — ⑤-b 가 먼저 쓰기 때문에 위로 올렸다.)
_LEGACY=""
_BD=$(_cys_dirpart "${BASH_SOURCE:-}") || _BD=""
if [ -n "$_BD" ] && [ -f "$_BD/role-bootstrap-legacy.sh" ]; then
  _LEGACY="$_BD/role-bootstrap-legacy.sh"
fi
if [ -z "$_LEGACY" ]; then
  _AD=$(_cys_dirpart "${0:-}") || _AD=""
  if [ -n "$_AD" ] && [ -f "$_AD/role-bootstrap-legacy.sh" ]; then
    _LEGACY="$_AD/role-bootstrap-legacy.sh"
  fi
fi
if [ -z "$_LEGACY" ] && [ -f "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/role-bootstrap-legacy.sh" ]; then
  _LEGACY="${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/role-bootstrap-legacy.sh"
fi
if [ ! -f "${_LEGACY:-}" ]; then
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
# ★⑤-b 발급 고지는 exec **전에** 낸다 — exec 뒤에는 이 런처가 아무것도 쓸 수 없다. 단 본체가 자기
#   고지를 낼 수 있는 입력(마스터 토큰 보유 — 본체의 고지는 전부 선언 감지 또는 `_maybe_declaration`
#   술어 뒤에서만 나간다)이면 두 줄이 될 수 있으므로 **거부·판정 불가 고지는** 싣지 않는다(거부 사실은
#   원장 issue_refused 에 남는다). 술어는 본체 `_maybe_declaration` 과 같이 훅 입력 **전문**을 보되 그
#   상위집합이다(감지기의 영문 선언은 대소문자 무시 · JSON \u 이스케이프 16진 대소문자 혼용까지) — 좁으면
#   두 줄이 새고, 넓으면 거부 고지 하나를 더 잃을 뿐이다.
#   ★발급 고지(토큰)는 빼지 않는다: 승인 발화는 화이트리스트 전문 일치라 선언일 수 없어 본체의 선언
#   고지와 겹치지 않는다. 전문 술어가 cwd·transcript_path 의 'master' 글자에 걸린다는 이유로 토큰을
#   버리면 그런 폴더의 오너는 대화 승인이 영영 안 된다(겹침이 남는 곳은 본체가 판정 불가 강등 고지를
#   내는 고장 상태뿐이다 — 설계 보고서 잔여 위험에 적었다).
if [ -n "$_CYS_TT_NOTE" ]; then
  if [ "${_CYS_TT_KIND:-}" != "issued" ]; then
    case "$(cat "$IN" 2>/dev/null)" in
      *마스터*|*[Mm][Aa][Ss][Tt][Ee][Rr]*|*'\u'[bB]9[cC]8'\u'[cC]2[aA]4'\u'[dD]130*)
        echo "[cys-hook] role-bootstrap: 대화 승인 거부 고지 생략(마스터 토큰 입력 — 본체 고지와 두 줄 방지 · 원장 issue_refused 에 기록됨): $_CYS_TT_NOTE" >&2
        _CYS_TT_NOTE="" ;;
    esac
  fi
  _cys_tt_flush
fi
exec "$_CYS_SH" "$_LEGACY" "$IN"
