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
# 로드 시 자동 적용 (부작용 없음·stdout 무출력)
# ─────────────────────────────────────────────────────────────────────────────
cys_resolve_py >/dev/null 2>&1 || :
cys_fix_locale >/dev/null 2>&1 || :

# ★반드시 0으로 끝난다(계약 ⓓ) — 비0이면 호출측 loud-skip이 오발동한다.
:
