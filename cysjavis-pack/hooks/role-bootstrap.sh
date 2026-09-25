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
# ★(0.14.42 리뷰 RR1-SEC-B) 이름에 **좌석**을 싣는다 — 대화 승인 발급기(⑤-b · javis_teamtoken.py issue)는 이름의
#   좌석이 부르는 좌석(env)과 같아야 판정한다(다른 좌석에서 오너가 친 진짜 승인 입력을 이 좌석의 질문에 재생하는 길을
#   막는다 · 아래 GC 가 최근 20개를 남긴다). 좌석 표기는 `12`·`surface:12` 둘이다(javis_bootstrap.my_surface_key 가
#   숫자부로 흡수) — 셸 확장만으로(외부 명령 0) 콜론 앞을 걷고, 숫자가 아니면 `x`(발급기가 이름 형식 위반으로 거부 =
#   발급 0 · fail-closed). 부트 본체·신 파이프라인은 경로만 받고 이름을 해석하지 않는다.
_CYS_IN_SEAT="${CYS_SURFACE_ID:-${AITERM_SURFACE_ID:-}}"
_CYS_IN_SEAT="${_CYS_IN_SEAT##*:}"
case "$_CYS_IN_SEAT" in ''|*[!0-9]*) _CYS_IN_SEAT=x ;; esac
IN="$STATE/hook-input-$_CYS_IN_SEAT-$$.json"
if ! cat > "$IN" 2>/dev/null; then
  rm -f "$IN" 2>/dev/null
  _cys_note "[cys-hook] 훅 입력 저장 실패 — 부트 미발화"
  exit 0
fi

# 유계 GC(T3-4 정신) — 비정상 종료로 남은 입력 파일이 무한 누적되지 않게 최신 20개만 남긴다.
# 파이프 while 을 쓰는 이유: 경로에 공백이 있어도 단어분리로 깨지지 않는다.
# ★(fatal-fix R1-02 · R1-01 ⓑ) 상태 폴더는 **전 레인·전 좌석이 함께 쓴다**. 종전 GC 는 동시 훅이 20개를 넘으면(정각 정렬
#   방송·스케줄 · 좌석 최대 약 45) **다른 좌석이 아직 처리 중인** 입력을 지웠다 — 그 좌석의 본체는 stdin 이 이미 소진돼
#   INPUT 이 빈 채 무음 exit 0(부트 선언 감지·오너 임무 기록·기계 유래 처리 무음 누락), 발급기는 not_hook_caller(v0.14.41 부터).
#   이름의 끝 숫자는 그 입력을 쓴 런처의 pid 다(`$$` — 본체 exec 도 같은 pid 를 쓴다): **살아 있으면 건너뛴다**
#   (`kill -0` 은 셸 내장 · 외부 명령 0). 상한: 최신 200개 밖은 살아 있어도 지운다(pid 재사용·판정 이상이 무한 누적이 되지
#   않게). 실패 방향: kill -0 이 늘 실패하면 종전 거동(삭제) · 늘 성공하면 200개 상한 — 어느 쪽도 새 무한 누적 없음.
_cys_gc_n=20
# ★(0.14.42 R3-4 · C3) 20개 이하면 아래 파이프라인(ls·tail 외부 2 + 서브셸 · 맥 실측 약 4ms · Git Bash 는 더 비싸다)을
#   띄우지 않는다 — 셸 글롭으로 개수만 센다(외부 명령 0). 동치: 파이프라인은 최신 20개 **밖**만 지우므로 20개 이하에서는
#   원래 아무것도 하지 않는다. 두 쪽 모두 같은 글롭(`hook-input-*.json`)에 기대므로 글롭이 꺼진 셸에서도 거동이 같다.
#   실패 방향: 세는 사이 파일이 늘면 이번 한 번 건너뛸 뿐 다음 런처가 치운다(새 무한 누적 없음). 글롭 불일치(0개)는
#   패턴 자체 1개로 남아 `-gt 20` 이 거짓이다. 함수 안의 `set --` 라 런처의 "$@" 는 무접촉(dash·bash 같음).
#   이 게이트는 Windows 갈래에도 적용된다(내장 명령만 · 20개 이하 갈래는 windows-health 의 런처 실행 검체가 지난다).
_cys_gc_due() {
  set -- "$STATE"/hook-input-*.json
  [ "$#" -gt 20 ]
}
if _cys_gc_due; then
ls -1t "$STATE"/hook-input-*.json 2>/dev/null | tail -n +21 | while IFS= read -r _old; do
  _cys_gc_n=$((_cys_gc_n + 1))
  if [ "$_cys_gc_n" -le 200 ]; then
    _cys_gc_p="${_old##*-}"
    _cys_gc_p="${_cys_gc_p%.json}"
    case "$_cys_gc_p" in
      ''|*[!0-9]*) ;;
      *) kill -0 "$_cys_gc_p" 2>/dev/null && continue ;;
    esac
  fi
  rm -f "$_old" 2>/dev/null
done
fi

# ── ⑤-b 대화 승인 1회용 팀 생성 토큰 발급(0.14.42 · 설계 §6-4·§6-6·§7-3·§11 R11·§13 P4) ────────
# 오너가 master 의 질문("이 내용으로 만들까요?")에 이 좌석에서 직접 "만들어"라고 치면, 그 프롬프트에서
# 1회용 토큰을 발급한다. 판정·발급은 `bin/javis_teamtoken.py issue` 가 단일 소유하고(배달 원장 대조로
# 오너 실키 입력을 가린다 — 기계 배달엔 발급하지 않는다), 인터프리터가 필요한 호출은 프리루드 소비
# 파일 `teamtoken-issue.sh` 가 한다(이 런처는 프리루드 규약 심볼을 쓰지 않는다 — 자기완결 계약).
# ★자리가 여기인 이유: 아래 ⑥ 은 기계 유래 프롬프트를 처리완료(rc 6)로 닫아 본체를 건너뛴다. 발급
#   호출이 본체에 있으면 바로 그 경로(기계 배달된 "그래 만들어")에서 거부 사유가 원장에 남지 않는다.
#   모든 프롬프트가 지나는 한 점은 여기다.
# ★비용 게이트(매 프롬프트 공통 — 인터프리터를 띄우기 **전**에 거른다 · 리뷰 F3 개정):
#   **이 좌석의 열린 질문 표지**(`teamtoken-open-<레인>-s<좌석>` · 발급기 모듈이 원장에서 파생해 세우고 걷는 캐시)가
#   없다 = 이 좌석에 답을 기다리는 질문이 없다 → 셸 글롭 존재 검사 1회로 끝(외부 명령 0). 레인 키 규약을 여기 복사하지
#   않는다 — 레인 자리의 `*` 는 상위집합이라 안전하다. 표지는 질문(ask)이 열릴 때 **먼저** 세워지고(못 세우면 질문이
#   열리지 않는다), 답·만료로 닫히면 걷힌다. 답 없이 만료된 질문은 고지 여유(300초) 동안 남아 "확인 시간이 지나
#   다시 여쭙습니다"(§10)를 한 번 말해 주고, 그 뒤 첫 호출이 걷는다(묵은 표지 = 발급기 1회 뒤 0회).
#   종전(원장 mtime 10분 창)은 질문이 닫힌 뒤에도 · 거부 감사 레코드 한 줄에도 10분 동안 모든 좌석의 모든
#   프롬프트가 발급기를 띄웠고(맥 약 +130ms), 원장이 한 번 생긴 기계는 영구히 date·stat 2회(약 +8ms)를 냈다.
#   그 대가로 '질문 없이 승인처럼 들리는 말'의 ask_not_open 고지는 질문이 열린 동안에만 난다 — 그 밖은
#   master 의 `javis_teamtoken.py status` 가 보인다(§7-3 · 디렉티브 §4-A 절차 5).
#   ★게이트의 실패 방향: 표지를 잘못 걸러도 발급 0(= fail-closed)일 뿐 허용이 되지 못한다 — 판정은 언제나
#     발급기가 원장 내용으로 한다. 표지가 사라졌으면 status 가 원장에 맞춰 되살린다.
# ★실패 정책(이 런처와 같다): 발급기가 없거나 죽거나 늦어도 프롬프트 제출은 막지 않는다 — 발급기는
#   전경에서 돌되 stdio 를 이 훅에서 끊고(`</dev/null >/dev/null` · stderr 는 진단용으로 통과) 자기
#   데드라인(인터프리터 호출 6s + 고지 3s)으로 유계다. 결과는 고지 파일 1줄로만 돌아온다(파이프 점유 0).
#   ★(fatal-fix R3-F2 · R1-01 · R2-1 · WIN-3) 표지는 **좌석** 단위다(`teamtoken-open-<레인>-s<좌석>`) — 글롭은 이 좌석의
#     표지만 본다. 질문을 열 수 있는 좌석은 제안을 올린 대표 좌석 하나라, 종전 전 레인 글롭은 질문 창 동안 워커·리뷰어·
#     CSO·부서 좌석 전부에 인터프리터 1~2회를 붙였다(런처 GC 경합과 발급기 고장 잡음의 노출면). 좌석 표기는 ⑤ 의 입력 파일
#     이름과 같다(`_CYS_IN_SEAT` — 발급기의 좌석 결박이 이미 둘의 일치를 요구한다). 다른 레인의 같은 번호 좌석은 상위집합.
#   ★(fatal-fix R3-F7) 롤백 축(`CYS_BOOT_GATES=0` — 본체가 소비하는 기존 노브를 **읽기만** 한다)에서는 건너뛴다: 롤백 축의
#     훅 시한은 하네스 기본(30s)이라 본체 예산(27s)에 ⑤-b(최대 약 9.5s)가 더해지면 부트 고지가 무음 취소된다 · 사고 순간
#     대화 승인을 끄는 수단이기도 하다(발급 0 = fail-closed · 화면 경로는 그대로).
_cys_tt_open() {
  [ "${CYS_BOOT_GATES:-1}" = "0" ] && return 1
  for _tt_f in "$STATE"/teamtoken-open-*-s"$_CYS_IN_SEAT"; do
    [ -f "$_tt_f" ] && return 0
  done
  return 1
}
if _cys_tt_open; then
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
# ★(0.14.42 R3-4) 플랫폼 두 갈래 — **Windows(Git Bash · cygpath 실재)는 종전 ⑥ 을 바이트 그대로 돈다.**
#   이유: windows-health 실기 레인의 가짜 cys 는 `--input` 을 광고하지 않는다 — 이 위임 경로는 Windows CI 에서
#   **한 번도 실행되지 않는다**(프로브 음성 → 본체). 실기 증거 없는 변경을 Windows 설치본에 싣지 않는다.
#   `WIN-BEGIN`~`WIN-END` 사이는 6371f8bd 의 ⑥ 전문과 바이트 동일하다(검체 test_hook_r34 WIN-0 이 핀).
#   R3-4 는 맥·리눅스 갈래(`else`)에만 실린다. 판별은 이 런처가 이미 쓰는 술어(`command -v cygpath` · ④·⑥)와 같다.
if command -v cygpath >/dev/null 2>&1; then
# ── WIN-BEGIN(종전 ⑥ · 무변경) ──
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
# ── WIN-END ──
else
# ★(0.14.42 R3-4) 맥·리눅스 갈래 — 판정·rc 계약(6·3 만 건너뜀)·데드라인·T2-2(초과 = 비동기 계속) 의미는 종전과 같다.
#   바뀐 것 셋:
#   ① 완료 신호 = **자식 종료**(`kill -0` 실패 · 셸 내장). 종전은 RCF 의 **존재**를 완료로 읽었다. 그런데 RCF 는
#     자식이 파일을 '연 순간'(값을 쓰기 전) 보이고 — 빈 RC 는 `*)` 라 자식이 이미 처리(rc 6)한 선언을 본체가 한 번 더
#     처리한다(이중 처리) — 같은 이름의 **남의** RCF(pid 가 재사용된 이전 런처의 T2-2 잔재 · 늦은 기록)도 완료로 읽는다
#     (낡은 6/3 이면 이 프롬프트가 어느 경로도 타지 않는다). 자식이 끝났다면 자식의 마지막 명령인 RCF 쓰기(`>` 는
#     절단 후 기록)도 끝났으므로 값은 완결이고 **이번 자식의 값**이다. 새 대기는 없다 — 판정은 데드라인 루프 안의
#     `kill -0` 하나라 시한이 종전처럼 선다(무상한 `wait` 를 쓰지 않는다 · 늦은 기록자 검체 RCF-4).
#   ② 폴링 간격 = 0.01 → 0.02 → 0.03 → 0.04s(누계 0.10s = 종전 한 칸), 그 뒤 종전 0.1s(회계 1/100초 · 잘게 보는 틱 최대 4회).
#     종전 첫 확인은 자식 기동 **직후**라 늘 헛치고 곧장 0.1s 를 잤다 — 수 ms 에 끝나는 자식에도 100ms 이상을 썼다
#     (S36 좌석 400/400 이 100~199ms 칸). 잘게 보는 구간을 종전 한 칸(0.1s)에 맞춰 끝내므로 그 뒤 확인 격자는 종전과 간격이
#     같고 위상만 틱 오버헤드(o · 프로세스 기동) 3회분 뒤로 밀린다 — 느린 자식(0.1s 이상)에서 종전보다 늦어도 **최대 3o**
#     (맥 실측 o 약 6ms → 약 18ms)이고 평균 검출 지연은 종전과 같다. 누계를 0.1s 에서 어긋나게 두면 특정 길이의 자식에서
#     종전보다 격자 한 칸(약 0.1s) 늦는 창이 생긴다(실측: 기하 0.01→0.08 은 80~95ms 자식 p50 +55~70ms · 누계 0.07s 는
#     95ms 자식 +87ms — 설계 보고서). 외부 `sleep` 증가는 런처당 최대 3개로 유계다(종전 대비 · 좌석 45개 동시 = 최대 약
#     135개 · 0.1s 안).
#   ③ 낡은 RCF 선제거 — **있을 때만** rm(흔한 경우 외부 명령 0). 자식이 rc 를 못 남기고 죽는 드문 갈래에서 남의 값을
#     읽지 않게 한다(정상 갈래는 자식의 `>` 가 절단하므로 이 줄 없이도 이번 값이다).
#   실패 방향: sleep 이 소수 초를 모르면 1초 폴링(종전과 같은 회계) · 자식이 rc 없이 죽으면 종전처럼 T2-2 고지(본체 0 —
#     절반 처리됐을 수 있는 선언을 본체가 다시 하지 않는다) · 빈 RC 를 남기고 죽으면 종전처럼 본체(`*)`).
# ★(0.14.42 R3-4 · C2) 능력 프로브 **양성 캐시** — 프롬프트마다 `cys --help | grep`(맥 실측 약 5~8ms · 외부 프로세스 2)를
#   띄우지 않는다. 키 = 해소된 cys 절대경로(캐시 파일 내용 전문 일치) + mtime(캐시 파일이 바이너리보다 **새것**이어야 적중 ·
#   `-nt` 는 셸 내장). 적중 경로의 비용은 `$(command -v cys)` 서브셸 1개뿐이다(외부 명령 0).
#   ★mtime 키의 한계(정직 표기): 같은 경로의 **제자리 교체**(개발 빌드 · cp)만 무효화한다. 설치기·압축 해제는 빌드 시각
#     mtime 을 보존하는 일이 흔해(새 바이너리가 캐시보다 '오래된' 것이 된다) 앱 업데이트에서는 캐시가 계속 적중한다.
#     그래서 안전성은 mtime 이 아니라 다음 둘에서 나온다 —
#     ① **양성만 적는다**: 캐시가 틀릴 수 있는 방향은 "위임 시도" 하나뿐이고, `--input` 을 모르는 CLI 는 rc 2 → 아래 `*)`
#        로 본체가 돈다(종전 폴백 그대로 · 무음 사망 없음 — 건너뛰기는 여전히 rc 6·3 뿐 · 검체 PC-4).
#     ② 음성(구 CLI)은 적지 않는다 — 구 CLI 는 매번 프로브한다(종전 거동 · PC-5).
#   이 갈래(맥·리눅스)에서만 쓴다 — Windows 갈래는 종전 프로브 그대로다. 새 노브는 만들지 않는다(머리 주석 계약).
#   캐시 파일(`$STATE/hook-probe-input.ok`)을 지우면 다음 프롬프트가 다시 프로브한다(끄는 스위치는 아니다 — 다시 생긴다).
_cys_probe_input() {
  _cpi_b="$(command -v cys 2>/dev/null)" || return 1
  _cpi_c="$STATE/hook-probe-input.ok"
  case "$_cpi_b" in
    /*)
      if [ -f "$_cpi_c" ] && [ "$_cpi_c" -nt "$_cpi_b" ]; then
        _cpi_l=""
        IFS= read -r _cpi_l 2>/dev/null < "$_cpi_c" || :
        [ "$_cpi_l" = "$_cpi_b" ] && return 0
      fi ;;
    *) _cpi_c="" ;;
  esac
  cys hook user-prompt-submit --help 2>/dev/null | grep -q -- '--input' || return 1
  [ -n "$_cpi_c" ] && { printf '%s\n' "$_cpi_b" > "$_cpi_c"; } 2>/dev/null
  return 0
}
if _cys_probe_input; then
  RCF="$IN.rc"
  [ -e "$RCF" ] && rm -f "$RCF" 2>/dev/null
  # 자식은 진입 즉시 stdio 를 끊는다 — 자식이 훅 stdout 파이프를 쥐면 사람의 프롬프트 제출이
  # 먹통이 된다(이 팩이 실제로 치른 사고 · 본체의 배경 통보기와 같은 규율).
  ( exec >/dev/null 2>&1 </dev/null
    cys hook user-prompt-submit --input "$IN"
    printf '%s' "$?" > "$RCF" ) &
  _bg=$!
  _lim=$((CYS_HOOK_INPUT_DEADLINE_S * 100))
  _i=0
  _t=1
  while [ "$_i" -lt "$_lim" ]; do
    kill -0 "$_bg" 2>/dev/null || break
    case "$_t" in
      1) sleep 0.01 2>/dev/null ;;
      2) sleep 0.02 2>/dev/null ;;
      3) sleep 0.03 2>/dev/null ;;
      4) sleep 0.04 2>/dev/null ;;
      *) sleep 0.1 2>/dev/null ;;
    esac || { sleep 1; _i=$((_i + 100 - _t)); }
    _i=$((_i + _t))
    case "$_t" in 1) _t=2 ;; 2) _t=3 ;; 3) _t=4 ;; *) _t=10 ;; esac
  done
  if [ -f "$RCF" ] && ! kill -0 "$_bg" 2>/dev/null; then
    RC=""
    IFS= read -r RC 2>/dev/null < "$RCF" || :
    rm -f "$RCF" 2>/dev/null
    # 6=처리완료 · 3=억제. **이 둘만** 본체를 건너뛴다(0 은 proceed 라 본체로 간다).
    # (⑤-b 의 발급 고지는 이 갈래에서 EXIT 트랩이 단독으로 낸다. 아래 줄은 WIN 갈래의 같은 줄과 글자까지 같다 —
    #  음성 대조 MUT-1 이 두 갈래를 한 번에 변조한다.)
    case "$RC" in
      6|3) rm -f "$IN" 2>/dev/null; exit 0 ;;
    esac
  else
    # T2-2: 데드라인 초과(자식 생존) 또는 rc 없이 끝난 자식 — **폴백이 아니라 비동기 계속**이다(종전과 같다).
    _cys_note "[cys-hook] 부트 등록이 지연되어 백그라운드로 계속합니다(진행은 boot-progress 로 보고됩니다)"
    exit 0
  fi
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
