#!/bin/sh
# _lib.sh — cysjavis 훅 공용 프리루드 (T-0147-7 W1a · 재감사 §3 CS-4① · RC4 소멸)
#
# 문제(RC4): 크로스컷 규약(surface 게이트·CYS_PY 해소·백슬래시 정규화·드라이브 절대경로 판정·
#   cygpath 변환·로케일·상태 경로)이 훅마다 재복붙되어 드리프트했다. 실측 결과 python3 참조 훅
#   28개 중 CYS_PY 해소는 5개뿐(G22), cwd 절대경로 게이트는 `/*` 전용이라 Windows `C:\` cwd를
#   공란화(G19), 경로 판정은 '/' 전용이라 백슬래시 무음 우회(G18·G20·G23)였다.
#   → 규약은 파일마다 살지 않고 **여기 한 곳**에 산다.
#
# 계약(재감사 CS-4① 비평2 D-1 '프리루드 견고성 판정' 명문):
#   ⓐ **POSIX sh 호환** — 훅 shebang이 sh/bash 혼재라 bashism 금지(배열·`${x:0:n}`·`[[`·
#      `local`·`+=` 금지). 문자열 슬라이스는 CS-8 python 창 판정으로 이관(G25).
#   ⓑ **stdout 무출력** — 여러 훅(SessionStart·UserPromptSubmit)의 stdout은 모델 컨텍스트로
#      주입된다. 프리루드가 한 글자라도 stdout에 쓰면 전 훅의 출력 계약이 깨진다.
#   ⓒ **`set -u` 안전** — guard.sh·pre-dispatch.sh·actprobe-kill-gate.sh는 set -u다.
#      모든 변수 읽기는 `${X:-}` 형태만 쓴다.
#   ⓓ **멱등·항상 0으로 종료** — source 결과가 비0이면 호출측 loud-skip이 오발동한다.
#   ⓔ source 실패는 호출측이 **loud-skip**(stderr 1줄 + exit 0)으로 처리한다. 조용한 꺼짐도,
#      훅 전멸도 아닌 제3의 길. 규약 문장(전 훅 동일 · **2단 해소 후 loud-skip**):
#        . "$(dirname "$0")/_lib.sh" 2>/dev/null \
#          || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
#          || { echo "[cys-hook] _lib.sh 소실 — 훅 강등" >&2; exit 0; }
#      (하위 디렉터리 훅은 1단이 `$(dirname "$0")/../_lib.sh`.)
#      ★2단(팩 경로) 폴백이 필수인 이유(실측 회귀): 훅은 **팩 밖으로 복사돼 실행되는 경로가
#      실재한다** — 배선 하네스·테스트 스텁·`~/.cys/local/hooks/<이벤트>.d/` 오버레이가 훅 파일만
#      다른 디렉터리에 두고 돌린다. 1단만 두면 그 전부가 조용히(정확히는 stderr 1줄 남기고)
#      전면 강등된다(test_pre_dispatch.sh 하네스에서 56/56 → 10/56 로 실측 재현).
#      팩 경로는 레인을 존중한다(CYS_PACK_DIR) — 부서 레인이 base 프리루드를 집지 않는다.
#      ★팩 경로 env 정본 목록(A11 · T-0147-7 W3): CYS_PACK_DIR JAVIS_PACK_DIR AITERM_PACK_DIR AITERM_JARVIS_DIR
#      (src/pack.rs `PACK_DIR_ENV_KEYS` = javis_preflight·javis_bootstrap·javis_report·
#       javis_orchestra·javis_todo_stamp 의 `PACK_DIR_ENV_KEYS` 와 동일 목록·동일 순서).
#      훅은 그중 **CYS_PACK_DIR 하나만** 해소한다 — 의도적 축소다: 훅 env 는 데몬이 pane 에
#      주입하고(레인 스코프) 레거시 키는 사람이 셸에 export 하는 이주 경로라 훅 계약이 아니다.
#      이 목록이 코드와 갈리면 tests/test_todo_shared_constants.py 가 멈춘다(문서-코드 결박).
#   ⓕ 설치 건강성(프리루드 실재)은 preflight 핀 체크가 담당한다 — 런타임 loud-skip과 이중 방어.
#
# ★이 파일은 훅이 아니다 — settings.json에 등록하지 않는다(SELFCORR_HOOKS 명시 목록 방식이라
#   글롭 오등록 위험 없음). 파일명 접두 `_`는 '훅 아님'의 시각 신호다.

# 멱등 가드 — 중첩 source(pre-dispatch → 서브훅)에서 재정의 비용을 없앤다.
[ -n "${CYS_LIB_SH_LOADED:-}" ] && return 0
CYS_LIB_SH_LOADED=1

# ─────────────────────────────────────────────────────────────────────────────
# 1. surface 이중 게이트 (A2 — 오발화 차단)
# ─────────────────────────────────────────────────────────────────────────────
# cysd가 pane에 주입하는 CYS_SURFACE_ID(구 AITERM_SURFACE_ID) 둘 다 없으면 = cys 밖의 임의
# claude 세션(VS Code·일반 터미널)이다. 그런 곳에서 cys 부트를 발화하면 preflight 변형·데몬
# autostart·boot-last 오염이라는 부작용을 남긴다(A2 재검증: 추가 피해 3건).
# ※경계: cys pane 안에서 사람이 직접 타이핑하는 경우는 CYS_SURFACE_ID가 **있으므로 통과**한다
#   (T-0147-1 레거시 직접타이핑 경로와 정합) — 차단 대상은 비-cys 터미널뿐이다.
cys_require_surface() {
  [ -n "${CYS_SURFACE_ID:-}" ] && return 0
  [ -n "${AITERM_SURFACE_ID:-}" ] && return 0
  exit 0
}

# 판정만 필요한 곳(조건 분기)용 — exit 하지 않는 술어형.
cys_have_surface() {
  [ -n "${CYS_SURFACE_ID:-}" ] && return 0
  [ -n "${AITERM_SURFACE_ID:-}" ] && return 0
  return 1
}

# ─────────────────────────────────────────────────────────────────────────────
# 2. 경로 정규화 (G18·G19·G20·G23)
# ─────────────────────────────────────────────────────────────────────────────
# 백슬래시 → 슬래시. python측 술어(javis_bootstrap._socket_dept/_pack_dept·
# javis_scrub 등)가 전부 `.replace("\\","/")` 후 판정하므로 셸도 같은 정규화를 쓴다
# (셸↔python 판정 일치 = H-PRED-6/H-WIN-3 parity 검체의 대상).
cys_norm_path() {
  printf '%s' "${1:-}" | tr '\\' '/'
}

# 절대경로 판정 — POSIX `/…`·UNC `//…`(정규화 후) + Windows 드라이브 `C:…`.
# 종전 `case "$CWD" in /*)` 는 `C:\Users\x` 를 상대경로로 보고 CWD를 공란화해 SESSION_STATE
# 상향탐색·write-ahead·reflect를 Windows에서 전면 불능화했다(G19).
cys_is_abs() {
  case "${1:-}" in
    /*) return 0 ;;
    [A-Za-z]:*) return 0 ;;
    '\'*) return 0 ;;
    *) return 1 ;;
  esac
}

# 훅 stdin의 cwd 필드 정규화 — 드라이브/UNC 경로만 슬래시화한다.
# ★POSIX 경로는 **무접촉**: 디렉터리 이름에 백슬래시가 든 정당한 unix 경로를 깨지 않는다
#   (Windows 전용 이득 vs POSIX 회귀 0의 보수적 경계).
cys_norm_cwd() {
  case "${1:-}" in
    [A-Za-z]:*|'\'*) cys_norm_path "${1:-}" ;;
    *) printf '%s' "${1:-}" ;;
  esac
}

# 경로 접두 판정(정규화 후) — `pack-guard` 팩 소속·guard 자기보호 등에 쓴다(G23).
# 사용: cys_path_has_prefix "$FP" "$PACK"  → 0=접두 일치
cys_path_has_prefix() {
  _cys_p="$(cys_norm_path "${1:-}")"
  _cys_q="$(cys_norm_path "${2:-}")"
  case "$_cys_q" in */) _cys_q="${_cys_q%/}" ;; esac
  [ -n "$_cys_q" ] || return 1
  case "$_cys_p" in "$_cys_q"/*) return 0 ;; *) return 1 ;; esac
}

# ─────────────────────────────────────────────────────────────────────────────
# 3. cygpath 가드 (A6·G8)
# ─────────────────────────────────────────────────────────────────────────────
# Windows(PortableGit sh)에서 네이티브 python은 POSIX 경로(/c/…)를 열지 못한다. cygpath가
# 있으면 네이티브(윈도) 경로로 변환하고, 없으면(unix) 원본을 그대로 낸다 — 무변환이 정답.
# 종전 role-bootstrap.sh는 이 규약(inject-context.sh:38 선례)을 위반해 CYS_PACK_DIR 부재
# 폴백 경로에서 발화가 즉사하는데도 "발화됨"을 보고했다(A6).
cys_native_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "${1:-}" 2>/dev/null || printf '%s' "${1:-}"
  else
    printf '%s' "${1:-}"
  fi
}

# 안내 문자열용 셸 인용 — Windows 네이티브 경로(공백·백슬래시)를 그대로 붙이면 사용자가
# 복사해 실행할 수 없다(G8). 큰따옴표로 감싸고 내부 `"`·`\`·`$`·백틱만 이스케이프한다.
cys_shquote() {
  printf '"%s"' "$(printf '%s' "${1:-}" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/\$/\\$/g' -e 's/`/\\`/g')"
}

# ─────────────────────────────────────────────────────────────────────────────
# 4. CYS_PY 해소 (G22 — python3 경성 참조 소멸)
# ─────────────────────────────────────────────────────────────────────────────
# Windows에는 `python3` 명령이 없고 `python`/`py`만 있는 경우가 흔하다. 해소 실패 시
# **빈 문자열**을 남긴다 — 호출측이 'cannot-judge' 로 loud 분기할 수 있게(판정불가와
# 판정-아님의 융합 금지 · CS-2⑨). 종전처럼 `|| echo python3` 로 채우면 '존재하지 않는
# 인터프리터'가 마치 해소된 것처럼 보여 실패 원인이 소실된다.
# ※기존 계약 보존: 비어 있으면 안 되는 호출부는 `[ -n "$CYS_PY" ] || CYS_PY=python3` 로
#   자기 자리에서 명시 폴백한다(계약 무변경 · 인터프리터 해소만 추가).
cys_resolve_py() {
  if [ -n "${CYS_PY:-}" ]; then
    if [ -x "${CYS_PY}" ] || command -v "${CYS_PY}" >/dev/null 2>&1; then
      export CYS_PY
      return 0
    fi
  fi
  CYS_PY="$(command -v python3 2>/dev/null || command -v python 2>/dev/null || command -v py 2>/dev/null || printf '%s' '')"
  export CYS_PY
  [ -n "${CYS_PY:-}" ]
}

# ─────────────────────────────────────────────────────────────────────────────
# 5. 로케일 고정 (G9 완화)
# ─────────────────────────────────────────────────────────────────────────────
# `grep -E '.{0,15}'` 은 로케일이 C면 **바이트**를 세므로 한글 filler 창이 약 1/3로 줄어
# 선언 감지가 미발화한다(role-bootstrap.sh:53 실측). 완전 해소는 감지기 python 이관(W1b·
# CS-8)이고, 여기서는 **LC_ALL 미설정 시에만** UTF-8 계열로 고정해 표면을 좁힌다.
# ※LC_ALL이 명시적으로 C인 환경은 사용자 의도로 존중한다(덮지 않는다 — 여기서 덮으면
#   사용자의 명시 설정을 훅이 무음 변경하는 더 나쁜 결함이 된다).
_cys_locale_exists() {
  command -v locale >/dev/null 2>&1 || return 1
  # 표기 차이 흡수(glibc `C.utf8` ↔ 요청 `C.UTF-8`): 소문자화 + `_`·`-` 제거 후 정확 비교.
  locale -a 2>/dev/null | tr 'A-Z' 'a-z' | tr -d '_-' \
    | grep -qxF "$(printf '%s' "${1:-}" | tr 'A-Z' 'a-z' | tr -d '_-')"
}

cys_fix_locale() {
  [ -n "${LC_ALL:-}" ] && return 0
  case "${LANG:-}" in
    *UTF-8*|*utf-8*|*UTF8*|*utf8*) LC_ALL="${LANG}"; export LC_ALL; return 0 ;;
  esac
  case "${LC_CTYPE:-}" in
    *UTF-8*|*utf-8*|*UTF8*|*utf8*) LC_ALL="${LC_CTYPE}"; export LC_ALL; return 0 ;;
  esac
  # LANG·LC_CTYPE 둘 다 UTF-8이 아닌 드문 환경만 실재 확인 후 고정(존재하지 않는 로케일을
  # 심으면 libc가 "C"로 되돌려 오히려 나빠진다 — 그래서 무조건 대입은 금지).
  for _cys_c in C.UTF-8 en_US.UTF-8; do
    if _cys_locale_exists "$_cys_c"; then
      LC_ALL="$_cys_c"; export LC_ALL
      return 0
    fi
  done
  return 0
}

# ─────────────────────────────────────────────────────────────────────────────
# 5-b. 데드라인 실행기 (A5 훅면 — 판정 호출의 hang 차단)
# ─────────────────────────────────────────────────────────────────────────────
# `cys surface-role` 같은 **판정 조회**가 데몬 미응답으로 행 걸면 훅이 사용자 프롬프트를
# 무한정 붙잡는다. GNU `timeout(1)`은 **macOS 기본 설치에 없다**(coreutils 미설치 기계) —
# session-start.sh:96-100 은 그래서 `command -v timeout` 분기를 쓰지만, 부재 시 그냥
# 타임아웃 없이 실행해 hang 표면을 그대로 남겼다. 여기서는 3단으로 해소한다:
#   ① timeout(1) → ② gtimeout(1)(coreutils) → ③ CYS_PY 기반 실행기(가장 흔한 macOS 경로)
#   ④ 셋 다 없으면 무-데드라인 직접 실행(정직한 강등 — 조용한 실패보다 낫다)
# 반환 코드는 실행 대상의 것을 그대로 전달하고, **타임아웃은 124**(GNU 관례)다.
# ★stdout 은 건드리지 않는다(호출측 `$(...)` 회수 계약 유지 · 프리루드 계약 ⓑ).
cys_timeout_run() {
  _cys_to="${1:-2}"
  shift
  # ★GNU 판별 선행: Windows PortableGit 은 System32 timeout.exe(인자 받으면 즉시 rc=1 함정 ·
  #   MEMORY cys-01411 #3)를 해소한다 — `--version` rc 0(GNU 계열)만 ①② 사용, 아니면 ③ 폴백.
  if command -v timeout >/dev/null 2>&1 && timeout --version >/dev/null 2>&1; then
    timeout "$_cys_to" "$@"
    return $?
  fi
  if command -v gtimeout >/dev/null 2>&1 && gtimeout --version >/dev/null 2>&1; then
    gtimeout "$_cys_to" "$@"
    return $?
  fi
  if [ -n "${CYS_PY:-}" ]; then
    # ★프로세스 **그룹** 종료: 대상만 죽이면 손자(대상이 띄운 자식)가 stdout 파이프를 계속 붙잡아
    #   호출측 `$(...)` 가 EOF 를 못 받고 그대로 행 걸린다(데드라인이 사실상 무효 — 실측 확인).
    #   그래서 새 세션으로 스폰하고 타임아웃 시 그룹 전체에 SIGKILL 을 보낸다.
    "$CYS_PY" -c 'import os,signal,subprocess,sys
try:
    p = subprocess.Popen(sys.argv[2:], start_new_session=True)
except FileNotFoundError:
    sys.exit(127)
except Exception:
    sys.exit(126)
try:
    sys.exit(p.wait(timeout=float(sys.argv[1])))
except subprocess.TimeoutExpired:
    try:
        os.killpg(os.getpgid(p.pid), signal.SIGKILL)
    except Exception:
        p.kill()
    sys.exit(124)' "$_cys_to" "$@"
    return $?
  fi
  "$@"
  return $?
}

# ─────────────────────────────────────────────────────────────────────────────
# 6. 상태 경로 (A17 훅면 — HOME 부재 backfill)
# ─────────────────────────────────────────────────────────────────────────────
# Windows 비로그인 셸(`bash -c`)은 HOME이 없어 `$HOME/.cys/state` 가 `/.cys/state` 로
# 붕괴한다. Rust측 spawn_env_pairs(HOME←USERPROFILE backfill)의 셸 대응물이다.
CYS_STATE_DIR="${CYS_STATE_DIR:-${HOME:-${USERPROFILE:-.}}/.cys/state}"
# ★네이티브 표기 정규화(2026-08-10 Windows 실기 H-MISSION-1 · run 31400677188): Git Bash(msys)는
#   기동 시 HOME 을 POSIX(/c/…)로 변환하므로 그 값으로 만든 CYS_STATE_DIR 은 POSIX 경로가 된다.
#   msys 는 네이티브 자식에게 PATH·HOME·TMP 류만 되변환하고 **커스텀 변수는 그대로** 넘기므로,
#   네이티브 python(javis_mission·javis_bootstrap — state_dir() 이 env 우선)이 `/c/…` 를
#   `C:\c\…` 로 오해석해 배달 원장·임무 대장을 통째로 못 봤다 — 층1(원장 해시 대조)이 무너져
#   라벨 없는 기계 배달이 오너 타이핑으로 오판되고 spawn 이 열렸다(§4-10 재개방).
#   cygpath 실재 환경(Windows)만 네이티브로 접고, unix 는 무변환이다(cys_native_path 계약).
CYS_STATE_DIR="$(cys_native_path "$CYS_STATE_DIR")"
export CYS_STATE_DIR

# ─────────────────────────────────────────────────────────────────────────────
# 7. 바이트코드 쓰기 봉인 (★SEAL-1 · 2026-08-01 실사고 근본원인)
# ─────────────────────────────────────────────────────────────────────────────
# macOS 앱 번들은 자기 안의 python 을 PATH 선두로 물린다(`Contents/Resources/runtime/
# python/bin`). 그 python 이 stdlib 을 import 하면 CPython 이 `__pycache__/*.pyc` 를
# **번들 안에** 새로 쓰고, 그 순간 코드서명 봉인이 깨진다("a sealed resource is missing
# or invalid / file added: …_compression.cpython-312.pyc"). 브라우저로 받은 사본은
# quarantine 이 붙어 첫 실행 때 Gatekeeper 전체 재검증에 걸리므로 → "손상되었기 때문에
# 열 수 없습니다"로 앱이 통째로 차단된다(공증·staple 은 정상이었다).
#
# 훅은 `$CYS_PY` 로 그 번들 python 을 부르는 최다 호출자다. Rust 층
# (`cys::spawn_env_pairs` — pane·스케줄 자식)이 이미 같은 쌍을 상속시키지만, 훅이 다른
# 경로(사용자가 직접 띄운 CLI 등)로 발화하면 그 상속을 못 받는다 → 여기서 한 번 더 잠근다.
# 값 규약: CPython 은 **비어 있지 않으면 참**이다. 빈 문자열은 "끔"이므로 `1` 고정
# (Rust 정본 = `src/lib.rs` `ENV_PY_NO_BYTECODE` · 대안 비교 근거도 그 주석에 있다).
# 무조건 export 인 이유: 실패 방향이 하나뿐이다 — 최악이 "매번 재컴파일"이고, 봉인은
# 절대 깨지지 않는다. PYTHONUTF8=1(cys-dept)과 같은 층위의 자식 전파 규약이다.
PYTHONDONTWRITEBYTECODE=1
export PYTHONDONTWRITEBYTECODE

# ─────────────────────────────────────────────────────────────────────────────
# 8. 레인 가드 (#16 · 2026-09-04 W-A)
# ─────────────────────────────────────────────────────────────────────────────
# 문제: 훅은 settings.json 의 **절대경로**로 등록된다. 부서 레인 pane(CYS_PACK_DIR=
# ~/.cys/pack-<부서>)에서 사용자 settings 가 본부 팩 훅을 가리키고 있으면, 본부 훅이 레인
# 안에서 발화해 **다른 팩의 규약**(디렉티브·soul·상태 경로)을 주입한다. 프리루드 2단 폴백이
# "팩 경로는 레인을 존중한다"고 선언한 것과 정반대 방향의 누수다.
#
# ★판별자는 **양쪽 대칭**이다(worker-4 실측 교정 · 이것이 없으면 사용자 오버레이가 죽는다):
#   ① `$CYS_PACK_DIR/hooks/_lib.sh` 실재  = 레인 쪽이 진짜 팩인가
#   ② `<훅 자기 루트>/hooks/_lib.sh` 실재 = 훅 쪽이 진짜 팩인가
#   ③ 정규화(`pwd -P`) 후 두 루트가 상이
# 셋이 모두 참일 때만 조기 종료한다. ②가 거짓이면 **판정 불능 → 통과**다 —
# `~/.cys/local/hooks/<이벤트>.d/` 오버레이·테스트 스텁·배선 하네스는 팩이 아닌 트리에서
# 돌면서 프리루드를 2단(팩 경로)으로 집는 **정당한 경로**이고(계약 ⓔ 실측), 그 디렉터리에는
# `_lib.sh` 가 없다. 한쪽만 보면 그 전부가 전면 무발동한다(업데이트 불가침 확장점 파괴).
# 막으려는 것은 '팩 밖 정당 실행'이 아니라 **다른 팩의 훅**이 레인에 끼어드는 경우다.
#
# 계약: POSIX sh · `set -u` 안전 · stdout 무출력 · 외부 명령 비의존(파라미터 확장 + cd/pwd
# 빌트인만 — PATH 가 빈 훅 하네스에서도 오판하지 않는다) · opt-out `CYS_HOOK_LANE_GUARD=0`.
cys_lane_guard() {
  [ "${CYS_HOOK_LANE_GUARD:-1}" = "0" ] && return 0
  [ -n "${CYS_PACK_DIR:-}" ] || return 0
  [ -f "${CYS_PACK_DIR}/hooks/_lib.sh" ] || return 0        # ① 레인 쪽이 진짜 팩인가

  # 훅 자기 루트: `$0` 의 디렉터리가 `hooks` 이거나 `hooks/<sub>` 일 때만 판정 가능.
  _cys_lg_d="${0%/*}"
  [ "$_cys_lg_d" = "${0:-}" ] && _cys_lg_d="."
  _cys_lg_d="$(cd "$_cys_lg_d" 2>/dev/null && pwd -P)" || _cys_lg_d=""
  [ -n "$_cys_lg_d" ] || return 0                            # 판정 불능 → 통과
  _cys_lg_root=""
  if [ "${_cys_lg_d##*/}" = "hooks" ]; then
    _cys_lg_root="${_cys_lg_d%/*}"
  else
    _cys_lg_p="${_cys_lg_d%/*}"
    [ "${_cys_lg_p##*/}" = "hooks" ] && _cys_lg_root="${_cys_lg_p%/*}"
  fi
  [ -n "$_cys_lg_root" ] || return 0                         # 판정 불능 → 통과
  [ -f "${_cys_lg_root}/hooks/_lib.sh" ] || return 0         # ② 훅 쪽이 진짜 팩인가(대칭)

  _cys_lg_lane="$(cd "${CYS_PACK_DIR}" 2>/dev/null && pwd -P)" || _cys_lg_lane=""
  [ -n "$_cys_lg_lane" ] || return 0                         # 판정 불능 → 통과
  [ "$_cys_lg_root" = "$_cys_lg_lane" ] && return 0          # ③ 같은 팩 → 통과

  # ★고지의 가시성 한계(정직 기록): 훅의 프리루드 규약 문장은 `. "…/_lib.sh" 2>/dev/null` 이라
  #   **source 명령 전체의 stderr 가 억제**된다 — 이 줄은 프리루드를 직접 로드하는 호출자
  #   (하네스·수동 진단)에게만 보인다. 훅 경로에서 관측 가능한 계약은 '무발화·exit 0·stdout 0'
  #   이며 검체 H-LANE-GUARD-1 이 두 사실을 각각 잰다(stderr 억제 자체는 이 변경 범위 밖이다).
  echo "[cys-hook] 타 레인 팩 훅 조기 종료(hook=$_cys_lg_root lane=$_cys_lg_lane)" >&2
  exit 0
}

# ─────────────────────────────────────────────────────────────────────────────
# 9. 좌석 역할 해소 — 데몬 권위 (0.14.31 P6 · 감사 codex E · R1 반영)
# ─────────────────────────────────────────────────────────────────────────────
# 정본 §8: "`CYS_ROLE` env 를 권위로 쓰지 않는다(승계 후 stale). 데몬 조회 우선."
# 승계(claim-role·takeover)는 데몬 roles 맵만 바꾸고 pane 의 env 는 **낡은 채로 남긴다** —
# 그 env 로 내리는 결정은 옛 신원의 결정이다. 파이썬 짝은 `bin/javis_role.py` 이며
# **같은 캐시 디렉터리·같은 레코드 문법**을 공유한다(tests/test_role_authority.py 가 결박).
#
# ★`role-capability-gate.sh` 는 이 함수를 쓰지 않는다 — 그 훅은 **능력 게이트**라
#   ⓐTTL 15s ⓑ'게이트 대상 캐시는 fast-path 아님' ⓒ후보가 갈리면 둘 다 적용(정책 교집합)
#   이라는 **더 엄격한 캐시 권위 경계**를 의도적으로 갖는다(그 파일 :43-60 주석). 여기 60s
#   일반 해소기로 갈아끼우면 그 경계가 조용히 헐거워진다. 두 해소기의 공존은 의도다.
#
# 계약(전 소비처 공통):
#   ⓐ rc 항상 0 · stdout 무출력(프리루드 계약 ⓑ) · `set -u` 안전 · POSIX sh
#   ⓑ 산출은 변수 두 개: `CYS_RESOLVED_ROLE`(역할 또는 빈 문자열) ·
#      `CYS_RESOLVED_ROLE_SOURCE` ∈ daemon|daemon-none|cache|cache-none|env-cys-role|none
#      (앞의 넷이 **권위 있는 답** · 뒤의 둘은 판정 불가 후의 현행 env 판정)
#   ⓒ **판정 불가(ⓒ상)와 권위 있는 무역할(ⓑ상)은 다른 사실이다** — 소비처가 source 로 가른다.
#      `cys surface-role`(src/bin/cys.rs:10940)이 rc 0=사실 / rc≠0=판정 불가로 이미 갈라 둔다.
#
# ★상수는 **고정**이다(R1 교정 · reviewer-claude): 종전에는 `CYS_ROLE_CACHE_TTL` 류 env 로
#   덮을 수 있었는데 파이썬 짝은 못 덮어서 "두 층 규칙 동일"이 거짓이었고, `..._TIMEOUT=0` 은
#   GNU `timeout 0`(무제한)까지 열었다. 노브를 없애는 쪽으로 통일한다(§3-4 "게이트를 끄는 노브 없음").
CYS_ROLE_CACHE_TTL=60        # 초 · 승계 반영 지연 상한(명시적 수용 · 파이썬 CACHE_TTL_S)
CYS_ROLE_QUERY_BACKOFF=30    # 초 · 조회 실패 후 재조회 유예(파이썬 FAIL_BACKOFF_S)
CYS_ROLE_QUERY_TIMEOUT=2     # 초 · 자식 데드라인(파이썬 QUERY_TIMEOUT_S)
CYS_ROLE_CACHE_DIRNAME="cys-role-authority.d"

# ASCII 공백 집합 — Rust `str::trim`(유니코드 공백까지) 의 **부분집합**(더 엄격한 쪽).
# 프리루드 로드 비용 0: 쓰는 순간 한 번만 만든다(훅은 초당 여러 번 뜬다).
cys_role_ws_init() {
  # 셋 다 있어야 초기화 완료다 — 하나만 ambient 로 들어와 있으면(`set -u`) 아래 참조가 죽는다.
  [ -n "${CYS_ROLE_WS:-}" ] && [ -n "${CYS_ROLE_NL:-}" ] && [ -n "${CYS_ROLE_CR:-}" ] && return 0
  CYS_ROLE_NL='
'
  CYS_ROLE_CR="$(printf '\r' 2>/dev/null)"
  CYS_ROLE_WS="$(printf ' \t\v\f' 2>/dev/null)${CYS_ROLE_CR}${CYS_ROLE_NL}"
  return 0
}

# 첫 줄만(CR·LF 어느 쪽이든 거기서 끊는다) — 외부 명령 0(head/tr/cut 부재 하네스에서도 동형).
cys_role_first_line() {
  cys_role_ws_init
  _cys_rf="${1-}"
  case "$_cys_rf" in *"$CYS_ROLE_NL"*) _cys_rf="${_cys_rf%%"$CYS_ROLE_NL"*}" ;; esac
  case "$_cys_rf" in *"$CYS_ROLE_CR"*) _cys_rf="${_cys_rf%%"$CYS_ROLE_CR"*}" ;; esac
  printf '%s' "$_cys_rf"
  return 0
}

# 양끝 ASCII 공백 트림 — 파라미터 확장만 쓴다.
cys_role_trim() {
  cys_role_ws_init
  _cys_rt="${1-}"
  # ★비용 상한: 1024자를 넘는 값은 어떤 토큰 문법(≤64)도 통과하지 못한다 — O(n) 문자 루프를
  #   돌지 않고 그대로 돌려준다(두 층의 최종 판정은 같다: 거절).
  [ "${#_cys_rt}" -gt 1024 ] && { printf '%s' "$_cys_rt"; return 0; }
  while [ -n "$_cys_rt" ]; do
    _cys_rt_c="${_cys_rt%"${_cys_rt#?}"}"
    case "$CYS_ROLE_WS" in *"$_cys_rt_c"*) _cys_rt="${_cys_rt#?}" ;; *) break ;; esac
  done
  while [ -n "$_cys_rt" ]; do
    _cys_rt_c="${_cys_rt#"${_cys_rt%?}"}"
    case "$CYS_ROLE_WS" in *"$_cys_rt_c"*) _cys_rt="${_cys_rt%?}" ;; *) break ;; esac
  done
  printf '%s' "$_cys_rt"
  return 0
}

# 역할 문자열의 **단일 정규화**(파이썬 `_line` 과 글자 그대로 같다): 첫 줄 + 트림.
cys_role_line() { cys_role_trim "$(cys_role_first_line "${1-}")"; return 0; }

# 레코드 토큰 문법 — 파이썬 `_TOKEN_RE` 와 같은 집합(**공백 불가** · 1~64자).
# 종전에는 셸이 역할 접미 공백까지 보존하고 파이썬은 strip 해서 같은 바이트가 `cso `/`cso` 로
# 갈렸다(reviewer-codex R1). 문법에서 공백을 아예 뺐다.
cys_role_token_ok() {
  case "${1-}" in ''|*[!A-Za-z0-9._:+-]*) return 1 ;; esac
  [ "${#1}" -le 64 ] || return 1
  return 0
}

# 파일명 성분 — 파이썬 `_slug` 와 **글자 그대로** 같다.
# ★R2(codex 위임 차분 프로브 실측): `tr -c` 만 쓰면 macOS `tr` 는 멀티바이트를 한 글자로 세고
#   파이썬은 UTF-8 바이트로 세어 **같은 소켓이 두 파일**이 됐다(R1 의 '바이트 단위로 같다'는
#   틀린 주장이었다). `-s`(연속 치환 접기)를 더하면 허용 집합이 순수 ASCII 라 두 층이 언제나
#   같은 구간을 접어 결과가 동일해지고, 결과가 ASCII 라 80자 절단의 단위 문제도 사라진다.
# ★I2 수렴(판정관 T2 · 2026-09-08): 허용 집합에서 `_` 를 **뺀다**. 종전 집합은 `_` 를 허용해서
#   입력에 원래 있던 `_` 를 파이썬은 보존하고 `tr -s` 는 출처를 가리지 않고 접었다
#   (`/tmp/a__b.sock` → py `_tmp_a__b.sock` vs sh `_tmp_a_b.sock` = 같은 종단점이 두 파일).
#   `_` 를 빼면 출력의 모든 `_` 가 치환 산물이라 두 층이 언제나 같은 구간을 접는다.
cys_role_slug() {
  _cys_sl="$(printf '%s' "${1-}" | tr -cs 'A-Za-z0-9.-' '_' 2>/dev/null)" || _cys_sl=""
  while [ "${#_cys_sl}" -gt 80 ]; do _cys_sl="${_cys_sl%?}"; done
  printf '%s' "$_cys_sl"
  return 0
}

# 신원/소켓 env — Rust `env_compat`(src/lib.rs:351)와 같은 우선순위·같은 '비어 있지 않음' 규칙.
cys_role_surface_env() { printf '%s' "${CYS_SURFACE_ID:-${JAVIS_SURFACE_ID:-${AITERM_SURFACE_ID:-}}}"; }
cys_role_socket_env()  { printf '%s' "${CYS_SOCKET:-${JAVIS_SOCKET:-${AITERM_SOCKET:-}}}"; }

# 데몬 신원 문자열 — 캐시 **레코드에 그대로 실려** 정확 비교된다(손실 슬러그가 서로 다른
# 소켓을 같은 키로 뭉개던 길 차단 · reviewer-codex R1). 지정이 없으면 상태 디렉터리·HOME 으로
# 문맥을 구분한다(Rust 기본 소켓이 그 둘에서 유도된다 · src/lib.rs:379).
#
# ★R2(major · reviewer-codex) — 두 가지를 한꺼번에 고친다.
#  ⓐ **명령 치환으로 회수하지 않는다.** `$(cys_role_sock_id)` 는 말미 개행을 먹어서
#     `CYS_SOCKET=/tmp/a.sock<LF>` 이 여기선 `/tmp/a.sock`(=다른 종단점의 신원)으로 접혔다 —
#     그러면 셸이 **남의 종단점 이름표를 단 레코드**를 쓰고 파이썬이 그것을 권위로 읽는다.
#     그래서 현재 셸에서 파라미터 확장만으로 계산해 `CYS_ROLE_SOCK_ID` 에 담는다.
#  ⓑ **자르지 않는다. 표현할 수 없으면 빈 값**(= 디스크 캐시 끔 · 매번 데몬 조회)이다:
#     절대 경로가 아닌 소켓(상대 경로·`C:foo`·named pipe — cwd·드라이브에 따라 다른 종단점) ·
#     `:` 가 든 XDG/HOME(접두 인코딩이 단사가 아니게 된다) · 512 초과 · LF/CR 포함.
#     파이썬 짝 `javis_role._sock_id` 와 **같은 규칙**이다(어긋나면 test_role_authority 가 멈춘다).
# ★I7 수렴(판정관 T5 · 2026-09-08 · codex 설계 비평 (i)(j)(k)) — 두 규칙을 파이썬 짝
#   `javis_role._is_abs_endpoint` / `_pct_esc` 와 **글자 그대로** 같게 둔다.
#  ⓐ Windows 정규 종단점은 named pipe 다(`bin/cys-dept:48` 이 `\\.\pipe\cys-dept-<n>` 을 만들고
#     `src/lib.rs:385` 의 기본 소켓이 `\\.\pipe\cys`). 종전 `/*` 검사는 그것을 통째로 '신원 미지'로
#     접어 **디스크 캐시도 `.fail` 백오프도 함께** 껐다 — 데몬이 죽으면 훅마다 2s 를 온전히 문다
#     (§7 ④ 방향이 Windows 에서만 사라진다). 이름이 비면·`/` 가 섞이면 인정하지 않는다.
#  ⓑ 기본 신원은 `:` 거절 대신 **퍼센트 이스케이프**로 단사가 된다(`%`→`%25` 먼저, `:`→`%3A`).
#     길이 접두를 쓰지 않는 이유: `${#var}` 가 dash 는 바이트·bash/zsh 는 글자라 UTF-8 HOME 에서
#     두 층이 갈린다(실측 `한글`: dash 6 · bash 2). 바꾸는 글자가 ASCII 둘뿐이라 비-ASCII 는
#     원문 바이트 그대로 복사된다.
cys_role_abs_endpoint() {   # $1=소켓 값 · rc 0 = cwd 에 매달리지 않는 종단점
  case "${1-}" in
    /*) return 0 ;;
    */*) return 1 ;;                       # `/` 가 든 값은 pipe 로 인정하지 않는다
    '\\.\pipe\'?*) return 0 ;;
    '\\?\pipe\'?*) return 0 ;;
    *) return 1 ;;
  esac
}

# ★결과는 `CYS_ROLE_PCT_OUT` 에 담는다 — **명령 치환으로 회수하지 않는다**(`$( )` 가 말미 개행을
#   먹어 다른 문맥의 이름표를 달던 R2 함정과 같은 자리). 현재 셸에서만 부른다.
cys_role_pct_esc() {   # $1=원문 → CYS_ROLE_PCT_OUT
  _cys_pe_r="${1-}"
  _cys_pe_o=""
  while :; do
    case "$_cys_pe_r" in
      *"%"*) _cys_pe_o="$_cys_pe_o${_cys_pe_r%%"%"*}%25"; _cys_pe_r="${_cys_pe_r#*"%"}" ;;
      *) _cys_pe_o="$_cys_pe_o$_cys_pe_r"; break ;;
    esac
  done
  _cys_pe_r="$_cys_pe_o"
  _cys_pe_o=""
  while :; do
    case "$_cys_pe_r" in
      *":"*) _cys_pe_o="$_cys_pe_o${_cys_pe_r%%":"*}%3A"; _cys_pe_r="${_cys_pe_r#*":"}" ;;
      *) _cys_pe_o="$_cys_pe_o$_cys_pe_r"; break ;;
    esac
  done
  CYS_ROLE_PCT_OUT="$_cys_pe_o"
  return 0
}

cys_role_sock_id_init() {
  cys_role_ws_init
  CYS_ROLE_SOCK_ID=""
  CYS_ROLE_PCT_OUT=""
  _cys_si="${CYS_SOCKET:-${JAVIS_SOCKET:-${AITERM_SOCKET:-}}}"
  if [ -n "$_cys_si" ]; then
    cys_role_abs_endpoint "$_cys_si" || return 0          # ⓐ 종단점이 아니면 신원 미지
  else
    cys_role_pct_esc "${XDG_STATE_HOME:-}"; _cys_si_x="$CYS_ROLE_PCT_OUT"
    cys_role_pct_esc "${HOME:-}"; _cys_si_h="$CYS_ROLE_PCT_OUT"
    _cys_si="default:$_cys_si_x:$_cys_si_h"               # ⓑ 단사 인코딩
  fi
  [ "${#_cys_si}" -le 512 ] || return 0
  case "$_cys_si" in *"$CYS_ROLE_NL"*|*"$CYS_ROLE_CR"*) return 0 ;; esac
  CYS_ROLE_SOCK_ID="$_cys_si"
  return 0
}

# 호환 표기(진단·검체용) — 계산은 위 함수가 하고 여기서는 값만 낸다.
cys_role_sock_id() { cys_role_sock_id_init; printf '%s' "$CYS_ROLE_SOCK_ID"; return 0; }

# 신원 — 파이썬 `surface_id()` 와 **같은 규칙**(값 전체 검사 · 선두 0 정규화 · 자릿수 19).
# 종전에는 첫 줄만 떼어 검사하고 원본을 CLI 에 넘겼다 → `"7\n junk"` 가 여기선 7 로 통과하는데
# Rust 는 파싱 실패로 rc0+빈 줄을 내서 **유효 surface 아래 '권위 무역할'을 캐시**했다(codex R1).
cys_role_surface_id() {
  _cys_sr="$(cys_role_surface_env)"
  [ -n "$_cys_sr" ] || return 1
  [ "${#_cys_sr}" -le 64 ] || return 1
  _cys_sr="$(cys_role_trim "$_cys_sr")"
  case "$_cys_sr" in surface:*) _cys_sr="${_cys_sr#surface:}" ;; esac
  case "$_cys_sr" in ''|*[!0-9]*) return 1 ;; esac
  [ "${#_cys_sr}" -le 19 ] || return 1
  while [ "${#_cys_sr}" -gt 1 ]; do
    case "$_cys_sr" in 0*) _cys_sr="${_cys_sr#0}" ;; *) break ;; esac
  done
  printf '%s' "$_cys_sr"
  return 0
}

# 전용 캐시 디렉터리(0700 · 소유자 자신 · group/other 쓰기 0) 또는 rc 1(캐시 사용 불가).
# ★tmp 루트에 파일을 흩뿌리지 않는다 — '예측 가능한 경로에 심어 둔 심링크·FIFO' 부류가
#   구조적으로 닫힌다(두 리뷰어 공통 지적). 검증 실패의 귀결은 **캐시 끔**(매번 데몬 조회)이지
#   남의 디렉터리 신뢰가 아니다.
# ★소유자 판정은 `ls -ldn` + `id -u` 로 한다 — `test -O` 는 POSIX 가 아니라 dash 에 없다
#   (있는 셸에서만 도는 검사를 두면 CI 셸에 따라 판정이 갈린다).
cys_role_cache_dir() {
  _cys_cb="${TMPDIR:-}"
  [ -n "$_cys_cb" ] || _cys_cb="${TEMP:-}"
  [ -n "$_cys_cb" ] || _cys_cb="${TMP:-}"
  [ -n "$_cys_cb" ] || _cys_cb="/tmp"
  # MSYS(Git Bash)에서 TEMP/TMP 는 `C:\...` 형식이라 셸 경로가 아니다 → /tmp 로 강등.
  case "$_cys_cb" in /*) : ;; *) _cys_cb="/tmp" ;; esac
  [ -d "$_cys_cb" ] || _cys_cb="/tmp"
  _cys_cd="$_cys_cb/$CYS_ROLE_CACHE_DIRNAME"
  [ -d "$_cys_cd" ] || mkdir -m 700 "$_cys_cd" 2>/dev/null || :
  [ -d "$_cys_cd" ] && [ ! -L "$_cys_cd" ] || return 1
  _cys_ls="$(ls -ldn "$_cys_cd" 2>/dev/null)" || _cys_ls=""
  [ -n "$_cys_ls" ] || return 1
  case "$_cys_ls" in d????-??-?*) : ;; *) return 1 ;; esac   # group/other 쓰기 0
  # (평시에는 `cys_resolve_role` 이 현재 셸에서 이미 세워 둔다 — 여기는 직접 호출용 폴백이고,
  #  서브셸에서 세운 값은 밖으로 나가지 않는다는 사실을 이 주석이 명시한다 · R2)
  [ -n "${CYS_ROLE_UID:-}" ] || CYS_ROLE_UID="$(id -u 2>/dev/null || printf '')"
  [ -n "$CYS_ROLE_UID" ] || return 1
  # ★필드 분해는 **noglob 안에서** 한다 — `ls -ldn` 마지막 필드는 경로이고, TMPDIR 에 `*`·`?`
  #   가 들어 있으면 비인용 확장이 파일명 확장을 일으켜 필드가 어긋난다.
  case "$-" in *f*) _cys_ng=1 ;; *) _cys_ng=0 ;; esac
  set -f
  set -- $_cys_ls
  [ "$_cys_ng" = "1" ] || set +f
  [ "${3:-}" = "$CYS_ROLE_UID" ] || return 1
  printf '%s' "$_cys_cd"
  return 0
}

# (surface, socket 슬러그)당 정확히 하나. boot-epoch 는 **레코드**에 있으므로 재기동 고아가 없다.
# ★신원이 빈 값(표현 불가)이면 경로 자체를 내지 않는다 — 호출측이 디스크 캐시를 끈다.
cys_role_cache_path() {
  [ -n "${CYS_ROLE_SOCK_ID:-}" ] || return 1
  _cys_cdp="$(cys_role_cache_dir)" || return 1
  printf '%s/role-%s-%s' "$_cys_cdp" "$(cys_role_slug "${1:-none}")" \
    "$(cys_role_slug "$CYS_ROLE_SOCK_ID")"
  return 0
}

# ★유계 판독(R2 major · reviewer-codex): 종전 `IFS= read -r x < "$f"` 는 **줄 전체를 다 읽은
#   뒤에** 길이를 쟀다 — 4KB 상한이 판독 뒤에 적용되니 상한이 아니었고, 자라는 파일은 판독을
#   붙잡았다. 지금은 `dd bs=4096 count=1`(POSIX · Git Bash 포함)로 **읽는 순간** 바이트를 묶고,
#   그 판독마저 `cys_timeout_run` 데드라인 안에서 돈다(`-f` 검사 뒤 FIFO 로 바꿔치기하는
#   경합에서 열기가 매달리는 것까지 유계로 만든다).
#   ★`dd` 가 없으면 **폴백하지 않고 판독 실패**로 낸다 — 무계 빌트인 판독으로 되돌아가면 이
#   수정이 그대로 원상복구된다(codex R2). 실패의 귀결은 캐시 미스(데몬 조회 1회)다.
#   ★정직한 비대칭: 셸 변수는 NUL 을 담지 못해 `c<NUL>so` 가 셸에선 `cso` 로 읽힌다(파이썬은
#     거절). 레코드를 쓸 수 있는 자는 NUL 없이도 같은 줄을 쓸 수 있으므로 공격력은 늘지 않고,
#     남는 것은 **두 층 판정이 갈릴 수 있다**는 파리티 한계다(노트 '알려진 비대칭'에 기록).
CYS_ROLE_READ_CAP=4096
cys_role_read_bounded() {   # $1=경로 · stdout: 선두 최대 4KB · rc≠0 = 판독 불가(캐시 미스)
  [ -n "${1:-}" ] || return 1
  [ -f "$1" ] && [ ! -L "$1" ] || return 1
  command -v dd >/dev/null 2>&1 || return 1
  cys_timeout_run "$CYS_ROLE_QUERY_TIMEOUT" \
    dd "if=$1" "bs=$CYS_ROLE_READ_CAP" count=1 2>/dev/null
  return $?
}

# boot-epoch 토큰 또는 `-`(모름). 파이썬 `_boot_epoch` 와 같은 규칙(권위가 아니라 세대 표식).
# ★I7 수렴(codex 설계 비평 (j)): `/` 로 시작하지 않는 종단점(named pipe)에서는 **아예 읽지
#   않는다** — 유닉스 `dirname '\\.\pipe\cys'` 는 `.` 이라 cwd 의 `boot-epoch` 를 열었고,
#   cwd 마다 다른 세대 표식이 레코드에 실려 같은 종단점의 캐시가 서로를 무효화한다.
#   파이썬 짝 `javis_role._boot_epoch` 와 같은 규칙(Windows 에서 이미 `-` 인 것과 같은 결과).
cys_role_epoch() {
  _cys_ep_s="$(cys_role_socket_env)"
  case "$_cys_ep_s" in /*) : ;; *) _cys_ep_s="" ;; esac
  if [ -n "$_cys_ep_s" ]; then
    _cys_ep_f="$(dirname "$_cys_ep_s" 2>/dev/null)/boot-epoch"
    if _cys_ep_l="$(cys_role_read_bounded "$_cys_ep_f")"; then
      _cys_ep_l="$(cys_role_line "${_cys_ep_l:-}")"
      if cys_role_token_ok "$_cys_ep_l"; then printf '%s' "$_cys_ep_l"; return 0; fi
    fi
  fi
  printf '%s' '-'
  return 0
}

# 레코드 문법 `"<ts> <role> <epoch> <sockid>"` 판독 — 파이썬 `_parse_record` 와 같은 규칙.
# 성공 시 CYS_ROLE_REC_TS / CYS_ROLE_REC_VAL 설정. 파일 종류 검사(`-f`·`! -L`)와 4KB 상한은
# `cys_role_read_bounded` 가 **판독 시점에** 집행한다(R2 — 종전엔 다 읽은 뒤에 길이를 쟀다).
cys_role_record() {   # $1=path $2=sockid $3=epoch
  [ -n "${1:-}" ] || return 1
  [ -n "${2:-}" ] || return 1          # 신원 미지 = 캐시 없음(빈 sockid 로 매칭되지 않게)
  _cys_rl="$(cys_role_read_bounded "$1")" || return 1
  _cys_rl="$(cys_role_first_line "${_cys_rl:-}")"
  [ -n "$_cys_rl" ] || return 1
  [ "${#_cys_rl}" -le 4096 ] || return 1
  _cys_r_ts="${_cys_rl%% *}"; _cys_r_1="${_cys_rl#* }"
  [ "$_cys_r_ts" = "$_cys_rl" ] && return 1
  _cys_r_ro="${_cys_r_1%% *}"; _cys_r_2="${_cys_r_1#* }"
  [ "$_cys_r_ro" = "$_cys_r_1" ] && return 1
  _cys_r_ep="${_cys_r_2%% *}"; _cys_r_sk="${_cys_r_2#* }"
  [ "$_cys_r_ep" = "$_cys_r_2" ] && return 1
  # ts: 선두 0 금지 · 1~12자리 — POSIX sh 산술이 8진수 해석("value too great for base")·
  # int64 초과로 죽던 길을 문법에서 잘라낸다(reviewer-codex R1).
  case "$_cys_r_ts" in ''|0*|*[!0-9]*) return 1 ;; esac
  [ "${#_cys_r_ts}" -le 12 ] || return 1
  cys_role_token_ok "$_cys_r_ro" || return 1
  cys_role_token_ok "$_cys_r_ep" || return 1
  [ "$_cys_r_sk" = "${2:-}" ] || return 1
  [ "$_cys_r_ep" = "${3:-}" ] || return 1
  CYS_ROLE_REC_TS="$_cys_r_ts"
  CYS_ROLE_REC_VAL="$_cys_r_ro"
  return 0
}

# 0600 · 같은 디렉터리 원자 교체. 실패는 무시(캐시는 최적화지 사실이 아니다).
# ★R2(major · reviewer-codex): 종전 `( set -C; … > "$1.$$.tmp" )` 는 안전하지 않았다 —
#   bash 의 noclobber 는 **정규 파일**만 거절하고 FIFO 는 그대로 연다. 그 자리에 읽는 쪽 없는
#   FIFO 를 심어 두면 `printf` 가 열기에서 영원히 멈추고, 이 쓰기는 데몬 조회의 2s 데드라인
#   **밖**이라 `cys-dept` 가 무한정 붙잡힌다.
#   지금은 **새 전용 디렉터리를 배타 생성**해 그 안에만 쓴다: 갓 만든 디렉터리에는 남이 심어 둔
#   것이 있을 수 없으므로 열기가 매달릴 대상 자체가 없다. `mktemp -d` 가 있으면 예측 불가한
#   이름을(선점 방지), 없으면 `mkdir -m 700`(원자·배타)로 만들고, **만들기에 실패하면 쓰지
#   않는다**(귀결은 데몬 조회 1회 더 · 오판이 아니다).
#   ★최종 배치 전에 대상이 디렉터리가 아님을 확인한다 — `mv` 는 디렉터리(그리고 디렉터리로 가는
#     심링크)를 **컨테이너로 취급**해 그 안으로 넣는다(파이썬 `os.replace` 와 다른 지점 · codex R2).
cys_role_cache_write() {   # $1=path $2=한 줄 내용(개행 없이)
  [ -n "${1:-}" ] || return 0
  [ ! -d "$1" ] || return 0
  _cys_rr_td=""
  if command -v mktemp >/dev/null 2>&1; then
    _cys_rr_td="$(mktemp -d "$1.XXXXXX" 2>/dev/null)" || _cys_rr_td=""
  fi
  if [ -z "$_cys_rr_td" ]; then
    _cys_rr_td="$1.$$.d"
    mkdir -m 700 "$_cys_rr_td" 2>/dev/null || return 0
  fi
  if ( umask 077; printf '%s\n' "${2-}" > "$_cys_rr_td/r" ) 2>/dev/null && [ ! -d "$1" ]; then
    mv -f "$_cys_rr_td/r" "$1" 2>/dev/null || rm -f "$_cys_rr_td/r" 2>/dev/null || :
  else
    rm -f "$_cys_rr_td/r" 2>/dev/null || :
  fi
  rmdir "$_cys_rr_td" 2>/dev/null || :
  return 0
}

# ★폴백은 `CYS_ROLE` **하나뿐**이다(codex R1): `CYS_SURFACE_ROLE` 은
#   `role-capability-gate.sh` 가 해소 결과로 export 하는 **산출물**이지 신원 입력이 아니다.
#   일반 폴백에 넣으면 `CYS_SURFACE_ROLE=cso` + `CYS_ROLE=worker` 에서 **현행이 거부하던 것을
#   새로 허용**하게 된다 — "판정 불가면 현행 그대로"라는 약속이 거짓이 된다. 두 키를 함께 보는
#   소비처(`inject-context.sh`)는 자기 계약으로 직접 본다.
cys_role_env_fallback() {
  _cys_rr_v="$(cys_role_line "${CYS_ROLE:-}")"
  if [ -n "$_cys_rr_v" ]; then
    CYS_RESOLVED_ROLE="$_cys_rr_v"; CYS_RESOLVED_ROLE_SOURCE="env-cys-role"; return 0
  fi
  CYS_RESOLVED_ROLE=""; CYS_RESOLVED_ROLE_SOURCE="none"
  return 0
}

# ★`cys_resolve_role --no-cache`(I5 수렴 · codex 설계 비평 (g)): **디스크 역할 캐시를 권위로
#   읽지 않는다**. 새 허용(=stale `CYS_ROLE` 을 뒤집는 통과 판정)의 근거는 살아 있는 데몬의
#   직접 응답뿐이어야 한다 — 캐시 레코드는 같은 uid 의 아무 프로세스나 쓸 수 있으므로(게이트는
#   도구 호출만 본다) 그것을 통과 근거로 삼으면 위조 한 줄이 lifecycle mutation 을 연다.
#   `.fail` 백오프는 **그대로 존중한다**(데몬 사망 시 매 호출 2s 정지가 §7 ④ 방향이다) —
#   백오프에 걸리면 판정 불가로 강등되고 그 귀결은 종전 env 동작(거부 방향)이다.
#   파이썬 짝은 `javis_role.confirm_role_detail()` 이다.
cys_resolve_role() {
  CYS_RESOLVED_ROLE=""
  CYS_RESOLVED_ROLE_SOURCE="none"
  _cys_rr_trust=1
  [ "${1:-}" = "--no-cache" ] && _cys_rr_trust=0
  # ★현재 셸에서 한 번 — 아래 `case` 가 `$CYS_ROLE_NL` 을 직접 쓴다(하위 함수는 전부 `$( )`
  #   서브셸이라 거기서 설정된 값은 여기까지 오지 않는다 · `set -u` 안전 보장).
  cys_role_ws_init
  # ★신원 전제: 숫자 surface id 가 없으면 데몬에게 '나'를 물을 수 없다. 그때 Rust 는
  #   rc 0 + 빈 줄을 내는데, 그것을 '권위 있는 무역할'로 채택하면 **주소가 없다는 사실이
  #   역할이 없다는 판정으로 승격**된다(정상 위임 경로·하네스가 죽는다). 조회 자체를 안 한다.
  _cys_rr_sid="$(cys_role_surface_id)" || { cys_role_env_fallback; return 0; }
  [ -n "$_cys_rr_sid" ] || { cys_role_env_fallback; return 0; }

  _cys_rr_now="$(date +%s 2>/dev/null || printf '0')"
  case "$_cys_rr_now" in ''|0*|*[!0-9]*) _cys_rr_now=0 ;; esac
  [ "${#_cys_rr_now}" -le 12 ] || _cys_rr_now=0

  # ★신원은 **현재 셸에서** 계산한다(명령 치환이 말미 개행을 먹어 다른 종단점의 이름표를
  #   달던 길 차단 · R2). 빈 값 = 표현 불가 = 디스크 캐시 끔(데몬에 매번 묻는다).
  cys_role_sock_id_init
  _cys_rr_sock="$CYS_ROLE_SOCK_ID"
  _cys_rr_ep="$(cys_role_epoch)"
  # ★`id -u` 1회는 **여기서** 한다(R2 minor · reviewer-claude): 종전에는 두 겹 서브셸
  #   (`$(cys_role_cache_path …)` → `$(cys_role_cache_dir)`) 안에서 세워서 서브셸이 끝나면
  #   사라졌고, 주석이 약속한 '최초 1회'와 달리 **해소마다** 포크가 하나 더 들었다.
  [ -n "${CYS_ROLE_UID:-}" ] || CYS_ROLE_UID="$(id -u 2>/dev/null || printf '')"
  _cys_rr_cache=""
  if [ -n "$_cys_rr_sock" ]; then
    _cys_rr_cache="$(cys_role_cache_path "$_cys_rr_sid")" || _cys_rr_cache=""
  fi

  # ① 신선 캐시 — 미래 시각(시계 역행)은 신선이 아니다(그러면 캐시가 무기한 유효해진다).
  CYS_ROLE_REC_TS=""; CYS_ROLE_REC_VAL=""
  if [ "$_cys_rr_trust" = "1" ] && [ -n "$_cys_rr_cache" ] && [ "$_cys_rr_now" -gt 0 ] \
     && cys_role_record "$_cys_rr_cache" "$_cys_rr_sock" "$_cys_rr_ep" \
     && [ "$CYS_ROLE_REC_TS" -le "$_cys_rr_now" ] \
     && [ $(( _cys_rr_now - CYS_ROLE_REC_TS )) -lt "$CYS_ROLE_CACHE_TTL" ]; then
    if [ "$CYS_ROLE_REC_VAL" = "-" ]; then
      CYS_RESOLVED_ROLE=""; CYS_RESOLVED_ROLE_SOURCE="cache-none"; return 0
    fi
    CYS_RESOLVED_ROLE="$CYS_ROLE_REC_VAL"; CYS_RESOLVED_ROLE_SOURCE="cache"; return 0
  fi

  # ② 데몬 조회 — 실패 백오프 창 안이면 곧장 폴백(데몬 사망 시 매 호출 2s 정지가 전 pane 에
  #    걸리는 것이 봉인표 ④ 방향이다).
  _cys_rr_fail=""
  [ -n "$_cys_rr_cache" ] && _cys_rr_fail="$_cys_rr_cache.fail"
  _cys_rr_skip=0
  CYS_ROLE_REC_TS=""
  if [ -n "$_cys_rr_fail" ] && [ "$_cys_rr_now" -gt 0 ] \
     && cys_role_record "$_cys_rr_fail" "$_cys_rr_sock" "$_cys_rr_ep" \
     && [ "$CYS_ROLE_REC_TS" -le "$_cys_rr_now" ] \
     && [ $(( _cys_rr_now - CYS_ROLE_REC_TS )) -lt "$CYS_ROLE_QUERY_BACKOFF" ]; then
    _cys_rr_skip=1
  fi
  if [ "$_cys_rr_skip" = "0" ] && command -v "${CYS_BIN:-cys}" >/dev/null 2>&1; then
    # ★`CYS_NO_AUTOSTART=1`: 소켓이 없으면 `cys` 는 autostart 경로를 탄다(src/bin/cys.rs:2258).
    #   **역할을 묻는 행위가 데몬을 낳아서는 안 된다** — 특히 `cys-dept` 가드는 데몬이 아직
    #   없을 때 도는 경로다.
    #   (★함수 앞 `VAR=1 func` 은 셸마다 '호출 후에도 남는가/자식에게 export 되는가'가 갈린다 —
    #    서브셸 안에서 **명시적으로 export** 해 두 모호성을 한꺼번에 없앤다.)
    _cys_rr_out="$( CYS_NO_AUTOSTART=1; export CYS_NO_AUTOSTART
                    cys_timeout_run "$CYS_ROLE_QUERY_TIMEOUT" \
                      "${CYS_BIN:-cys}" surface-role 2>/dev/null )"
    _cys_rr_rc=$?
    _cys_rr_role="$(cys_role_line "$_cys_rr_out")"
    if [ "$_cys_rr_rc" -eq 0 ]; then
      if [ -n "$_cys_rr_role" ] && ! cys_role_token_ok "$_cys_rr_role"; then
        # 표현 불가한 역할(공백 포함·64자 초과·문법 밖)은 **판정 불가**다 — 잘라 쓰면 없는
        # 역할을 지어내는 것이고 캐시 문법도 깨진다. 파이썬 짝과 같은 규칙.
        _cys_rr_rc=1
      fi
    fi
    if [ "$_cys_rr_rc" -eq 0 ]; then
      if [ -n "$_cys_rr_role" ]; then
        cys_role_cache_write "$_cys_rr_cache" \
          "$_cys_rr_now $_cys_rr_role $_cys_rr_ep $_cys_rr_sock"
        [ -n "$_cys_rr_fail" ] && { rm -f "$_cys_rr_fail" 2>/dev/null || :; }
        CYS_RESOLVED_ROLE="$_cys_rr_role"; CYS_RESOLVED_ROLE_SOURCE="daemon"; return 0
      fi
      # 권위 있는 '역할 없음' — 옛 역할 캐시를 덮는다.
      cys_role_cache_write "$_cys_rr_cache" "$_cys_rr_now - $_cys_rr_ep $_cys_rr_sock"
      [ -n "$_cys_rr_fail" ] && { rm -f "$_cys_rr_fail" 2>/dev/null || :; }
      CYS_RESOLVED_ROLE=""; CYS_RESOLVED_ROLE_SOURCE="daemon-none"; return 0
    fi
    cys_role_cache_write "$_cys_rr_fail" "$_cys_rr_now - $_cys_rr_ep $_cys_rr_sock"
  fi
  # ③ 판정 불가 — 낡은 캐시는 쓰지 않는다(옛 역할이 무기한 사는 길). env 폴백 = 현행 동작.
  cys_role_env_fallback
  return 0
}

# ─────────────────────────────────────────────────────────────────────────────
# 로드 시 자동 적용 (부작용 없음·stdout 무출력)
# ─────────────────────────────────────────────────────────────────────────────
cys_resolve_py >/dev/null 2>&1 || :
cys_fix_locale >/dev/null 2>&1 || :
# ★레인 가드는 마지막이다 — 통과 판정이 나야 그 아래 훅 본체가 돈다(조기 종료는 exit 0).
cys_lane_guard

# ★반드시 0으로 끝난다(계약 ⓓ) — 비0이면 호출측 loud-skip이 오발동한다.
:
