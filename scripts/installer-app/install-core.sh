#!/bin/bash
# install-core.sh — 설치 도우미(Install cys.app)의 원자 설치 코어.
#
# ★로직 이중구현 금지: 원자 교체(ATOMIC-1)는 레포 정본 scripts/atomic_bundle.py(정본은 Rust
#   src/app_bundle.rs)를 **그대로** 부른다. 이 셸은 배선일 뿐이며 계약을 재구현하지 않는다.
#
# ★보안(LPE 방어): 이 스크립트도, 그것이 부르는 atomic_bundle.py 도 **자기 번들 내부**
#   (Contents/Resources)에서만 실행된다. 두 파일은 서명·공증된 설치 도우미 번들 안에 봉인돼 있어
#   설치 시점에 변조될 수 없다(installer.applescript 가 승격 전 자기 codesign 검증을 강제 —
#   봉인이 깨지면 실행 자체를 멈춘다). 쓰기 가능한 DMG 형제·임의 경로의 스크립트를 root 로
#   실행하는 경로는 존재하지 않는다.
#
# ★소스 진위 게이트(LPE 방어 · finding 3): SRC(설치할 cys.app)를 신뢰하기 **전에**, 그리고 그
#   번들 안의 어떤 코드(동봉 python 포함)를 실행하기 **전에**, 시스템 실 바이너리(codesign·spctl)로
#   ① 서명 무결 ② Team ID 고정(Q43YA2NMF9) ③ Apple 공증 수용을 검증한다. 이 순서가 핵심이다 —
#   소스의 python 을 먼저 실행하면 공격자 소스의 코드를 root 로 돌리게 되므로, 진위 검증은 반드시
#   python 호출보다 앞서야 한다. (deploy_gate.py 는 자기 빌드 산출물을 설치하므로 이 공증 게이트를
#   태우지 않는다 — 그래서 게이트는 공유 atomic_bundle.py 가 아니라 설치 도우미 전용인 이 셸에 둔다.)
#
# ★런타임: 설치 시점엔 시스템 /usr/bin/python3 가 CommandLineTools 셔틀 스텁일 수 있어(초보자 맥에
#   CLT 미설치 → 개발자도구 설치 프롬프트 후 비영 종료) 신뢰 불가. 대신 **진위 검증을 통과한 소스
#   cys.app 동봉 python**(Contents/Resources/runtime/python/bin/python3 · REQUIRED_RESOURCES 로 항상
#   존재)을 우선 사용하고, 없을 때만 /usr/bin/python3 로 폴백한다. atomic_bundle.py 는 표준 라이브러리
#   + libc.renamex_np·codesign·ditto 만 쓰므로 동봉 python 으로 충분하다.
#
# 사용: install-core.sh <SRC cys.app> <DEST cys.app>
# exit 0=원자 설치 완료 / 1=설치 실패(기존 번들 무접촉·권한 실패 시 applet 이 승격 재시도)
#      / 2=소스 아님 / 3=번들 내부 도구 결손 / 4=소스 진위 실패(변조·Team ID 불일치·미공증 → 승격 금지)
#      / 5=이 Mac 에서 소스 동봉 python 을 실행할 수 없음(CPU 아키텍처·macOS 버전 — 한국어 한 문장 안내 · 승격 금지)
set -euo pipefail

SRC="${1:?SRC (source cys.app) 필요}"
DEST="${2:?DEST (destination cys.app path) 필요}"
SRC="${SRC%/}"; DEST="${DEST%/}"

# 자기와 같은 디렉터리(=서명된 번들 내부 Contents/Resources)에서 정본 래퍼를 찾는다.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
WRAPPER="$SCRIPT_DIR/atomic_bundle.py"

[ -f "$WRAPPER" ] || { echo "✗ 번들 내부 원자 설치 도구 없음: $WRAPPER" >&2; exit 3; }
[ -d "$SRC/Contents" ] || { echo "✗ 소스가 .app 번들이 아님: $SRC" >&2; exit 2; }

# ── ★소스 진위 게이트 (동봉 python 실행 전 · 시스템 실 바이너리만 사용) ──
# 기존 인증서 고정: Developer ID Application: yoonsik choi (Q43YA2NMF9). 공격자는 cys 개인키가
# 없어 이 Team ID 로 서명할 수 없다 → Team ID 대조는 위조 불능 불변식이다.
EXPECTED_TEAM_ID="Q43YA2NMF9"
# ① 서명 무결 — 변조·반쪽 소스 차단(codesign 은 stub 세트가 아닌 실 바이너리라 stock macOS 존재).
/usr/bin/codesign --verify --strict "$SRC" >/dev/null 2>&1 || {
  echo "✗ 소스 서명 검증 실패(변조·손상 소스) — 설치 거부: $SRC" >&2; exit 4; }
# ② Team ID 고정 — 다른 개발자·ad-hoc(팀 없음) 소스 차단.
SRC_TEAM="$(/usr/bin/codesign -dv --verbose=4 "$SRC" 2>&1 | /usr/bin/sed -n 's/^TeamIdentifier=//p' | head -1)"
[ "$SRC_TEAM" = "$EXPECTED_TEAM_ID" ] || {
  echo "✗ 소스 Team ID 불일치(expected=$EXPECTED_TEAM_ID got=${SRC_TEAM:-none}) — 설치 거부: $SRC" >&2; exit 4; }
# ③ Apple 공증 수용 — 이 빌드가 공증됐는가(Gatekeeper 수용 · 스테이플 티켓으로 오프라인 판정).
/usr/sbin/spctl --assess --type execute "$SRC" >/dev/null 2>&1 || {
  echo "✗ 소스 Gatekeeper/공증 평가 거부(미공증·미신뢰 소스) — 설치 거부: $SRC" >&2; exit 4; }

# ── ★동봉 python 실행 가능성 검사 (0.15.0 · T11 · RULINGS BQ1(d) · 오너 지시 추가 11) ──
# 위치가 전부다: 진위 게이트 ①②③ **뒤**, 동봉 python 실행 **앞**. 이 검사는 python·node 파일을 **읽기만** 하고
#   시스템 실 바이너리(od·tr·sysctl·uname·sw_vers — 전부 절대경로)만 쓴다 — 소스 번들 안의 어떤 코드도 실행하지
#   않는다. 번역 실행은 Rosetta 2 판정에 시스템의 /usr/bin/true 하나뿐이다(CA3 · 파일 경로 추정 대신 실제로 돌려 본다).
# 왜: 이 Mac 에서 돌 수 없는 python(인텔 Mac 의 arm64 전용 · Rosetta 2 없는 Apple 실리콘의 x86_64 전용 ·
#   python 이 요구하는 것보다 낮은 macOS)을 exec 하면 "Bad CPU type in executable"·dyld 오류 원문이 그대로
#   사용자에게 뜬다. 대신 무엇이 문제이고 무엇을 하면 되는지 한국어 한 문장을 stderr 로 내고 exit 5 로 멈춘다
#   (설치 도우미는 exit 1 에서만 관리자 승격을 재시도한다 — 5 는 승격하지 않고 그 문장을 보여 준다).
# 헤더나 기계 사실(칩·macOS 버전·Rosetta 2)을 읽지 못하면 막지 않는다 — 종전 동작 그대로(CA3: 확실한 판정에서만 멈춘다).
# CPU 하위 유형 규칙은 게이트(scripts/check_macho_arch.py)와 같다: arm64e 만 있는 조각은 Apple 실리콘에서 제3자 코드로
#   실리지 않고, x86_64h 는 Haswell 이상 인텔에서만 돈다(이 Mac 의 hw.cpusubtype 이 8 이거나 모를 때만 인정).
# >>> T11-PY-RUNNABLE
ic_u8s() { /usr/bin/od -An -v -t u1 -j "$2" -N "$3" -- "$1" 2>/dev/null || true; }
ic_u32() {   # <파일> <오프셋> <le|be> → 10진수 (못 읽으면 rc 1)
  local b
  b=( $(ic_u8s "$1" "$2" 4) )
  [ "${#b[@]}" -eq 4 ] || return 1
  if [ "$3" = le ]; then echo $(( b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24) ))
  else echo $(( (b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3] )); fi
}
ic_arch() {   # <cputype> <cpusubtype> → 이름 — 하위 유형까지(능력 비트 0xff000000 은 버린다 · 게이트와 같은 규칙)
  local s=$(( $2 & 16777215 ))
  case "$1" in
    16777223) case "$s" in 3) echo x86_64 ;; 8) echo x86_64h ;; *) echo "x86_64/sub$s" ;; esac ;;
    16777228) case "$s" in 0) echo arm64 ;; 1) echo arm64v8 ;; 2) echo arm64e ;; *) echo "arm64/sub$s" ;; esac ;;
    *) echo "cpu$1" ;;
  esac
}
ic_slices() {   # <파일> → "<아키텍처>:<오프셋>" 줄들 (Mach-O 가 아니거나 못 읽으면 rc 1)
  local f="$1" le be n i e cpu sub off
  le=$(ic_u32 "$f" 0 le) || return 1
  be=$(ic_u32 "$f" 0 be) || return 1
  case "$le" in 4277009103|4277009102)            # 0xfeedfacf · 0xfeedface (단일 아키텍처)
    cpu=$(ic_u32 "$f" 4 le) || return 1; sub=$(ic_u32 "$f" 8 le) || return 1
    echo "$(ic_arch "$cpu" "$sub"):0"; return 0 ;; esac
  case "$be" in 3405691582|3405691583)            # 0xcafebabe · 0xcafebabf (유니버설)
    n=$(ic_u32 "$f" 4 be) || return 1
    { [ "$n" -ge 1 ] && [ "$n" -le 30 ]; } || return 1
    e=20; [ "$be" = 3405691583 ] && e=32
    i=0
    while [ "$i" -lt "$n" ]; do
      cpu=$(ic_u32 "$f" $((8 + i * e)) be) || return 1
      sub=$(ic_u32 "$f" $((8 + i * e + 4)) be) || return 1
      if [ "$e" = 20 ]; then off=$(ic_u32 "$f" $((8 + i * e + 8)) be) || return 1
      else off=$(ic_u32 "$f" $((8 + i * e + 12)) be) || return 1; fi
      echo "$(ic_arch "$cpu" "$sub"):$off"; i=$((i + 1))
    done
    return 0 ;; esac
  return 1
}
ic_minos() {   # <파일> <슬라이스 오프셋> → "X.Y.Z" (LC_BUILD_VERSION · LC_VERSION_MIN_MACOSX · 없으면 빈 출력)
  local f="$1" o="$2" magic hs ncmds pos cmd size v k=0
  magic=$(ic_u32 "$f" "$o" le) || return 0
  case "$magic" in 4277009103) hs=32 ;; 4277009102) hs=28 ;; *) return 0 ;; esac
  ncmds=$(ic_u32 "$f" $((o + 16)) le) || return 0
  pos=$((o + hs))
  while [ "$k" -lt "$ncmds" ] && [ "$k" -lt 512 ]; do
    cmd=$(ic_u32 "$f" "$pos" le) || return 0
    size=$(ic_u32 "$f" $((pos + 4)) le) || return 0
    [ "$size" -ge 8 ] || return 0
    case "$cmd" in
      50) v=$(ic_u32 "$f" $((pos + 12)) le) || return 0; echo "$((v >> 16)).$(((v >> 8) & 255)).$((v & 255))"; return 0 ;;
      36) v=$(ic_u32 "$f" $((pos + 8)) le) || return 0; echo "$((v >> 16)).$(((v >> 8) & 255)).$((v & 255))"; return 0 ;;
    esac
    pos=$((pos + size)); k=$((k + 1))
  done
  return 0
}
ic_minos_of() {   # <파일> <아키텍처> → 그 아키텍처 조각(없으면 첫 조각)의 최소 macOS (못 읽으면 빈 출력)
  local sl s first=""
  sl="$(ic_slices "$1")" || return 0
  for s in $sl; do
    [ -n "$first" ] || first="$s"
    if [ "${s%%:*}" = "$2" ]; then ic_minos "$1" "${s##*:}"; return 0; fi
  done
  [ -n "$first" ] && ic_minos "$1" "${first##*:}"
  return 0
}
ic_ver_lt() {   # <a> <b> → a < b 이면 rc 0 (숫자가 아니면 rc 1 = 막지 않음)
  local a b i x y
  a=( $(printf '%s' "$1" | /usr/bin/tr '.' ' ') ); b=( $(printf '%s' "$2" | /usr/bin/tr '.' ' ') )
  for i in 0 1 2; do
    x="${a[$i]:-0}"; y="${b[$i]:-0}"
    case "$x$y" in ''|*[!0-9]*) return 1 ;; esac
    [ "$x" -lt "$y" ] && return 0
    [ "$x" -gt "$y" ] && return 1
  done
  return 1
}
ic_machine() {   # → "<칩> <인텔 하위 유형> <macOS 버전>" (못 읽은 값은 '-') — 시스템 실 바이너리만, 절대경로로
  local hw="-" sub="-" os
  [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ] && hw=arm64
  if [ "$hw" = "-" ] && [ "$(/usr/bin/uname -m 2>/dev/null || true)" = x86_64 ]; then hw=x86_64; fi
  if [ "$hw" = x86_64 ]; then
    sub="$(/usr/sbin/sysctl -n hw.cpusubtype 2>/dev/null || true)"
    case "$sub" in ''|*[!0-9]*) sub="-" ;; esac
  fi
  os="$(/usr/bin/sw_vers -productVersion 2>/dev/null || true)"
  case "$os" in ''|*[!0-9.]*) os="-" ;; esac
  echo "$hw $sub $os"
}
ic_rosetta() {   # [번역해 볼 시스템 바이너리 — 기본 /usr/bin/true] → yes(번역이 실제로 돈다) · no(돌지 않는다) · -(판정 불가)
  # 시스템 코드만 번역해 본다(CA3). 그 바이너리에 x86_64 조각이 없으면 "번역 실패"가 Rosetta 부재를 뜻하지 않는다
  #   → 판정 불가(-) = 막지 않는다. 설치 도우미의 관리자 재시도에서는 root 로 돈다 — 시스템 바이너리만 실행한다.
  local t="${1:-/usr/bin/true}" sl
  [ -x /usr/bin/arch ] && [ -x "$t" ] || { echo "-"; return 0; }
  sl="$(ic_slices "$t")" || { echo "-"; return 0; }
  case " $(echo $sl) " in *" x86_64:"*|*" x86_64h:"*) ;; *) echo "-"; return 0 ;; esac
  if /usr/bin/arch -x86_64 "$t" >/dev/null 2>&1; then echo yes; else echo no; fi
}
# ic_python_runnable <python> <칩 arm64|x86_64|-> <인텔 하위 유형|-> <macOS 버전|-> <Rosetta 2 yes|no|probe|-> [node]
#   → 0 = 실행 가능 **또는 판정 불가**(막지 않는다 · 종전 동작) · 5 = 확실히 못 돈다(한국어 한 문장을 stderr 로).
ic_python_runnable() {
  local py="$1" hw="$2" hwsub="$3" os="$4" rosetta="$5" node="${6:-}" slices run="" s a has_x86=no has_arm=no pymin nodemin
  [ "$hw" = arm64 ] || [ "$hw" = x86_64 ] || return 0
  slices="$(ic_slices "$py")" || return 0
  for s in $slices; do
    a="${s%%:*}"
    case "$a" in x86_64*) has_x86=yes ;; arm64*) has_arm=yes ;; esac
    if [ -z "$run" ] && [ "$hw" = arm64 ]; then
      case "$a" in arm64|arm64v8) run="$s" ;; esac
    elif [ -z "$run" ]; then
      case "$a" in
        x86_64) run="$s" ;;
        x86_64h) case "$hwsub" in -|8) run="$s" ;; esac ;;
      esac
    fi
  done
  if [ -z "$run" ] && [ "$hw" = arm64 ] && [ "$has_x86" = yes ]; then
    [ "$rosetta" = probe ] && rosetta="$(ic_rosetta)"
    if [ "$rosetta" = no ]; then
      echo "이 설치 파일은 인텔 Mac용이라 Rosetta 2가 없는 이 Apple 실리콘(M1·M2 등) Mac에서는 설치할 수 없습니다 — www.cysinsight.com 에서 Apple 실리콘용 설치 파일(이름에 aarch64가 들어간 .dmg)을 받아 다시 설치해 주세요." >&2
      return 5
    fi
    for s in $slices; do case "${s%%:*}" in x86_64*) run="$s"; break ;; esac; done
  fi
  if [ -z "$run" ]; then
    if [ "$hw" = x86_64 ] && [ "$has_arm" = yes ] && [ "$has_x86" = no ]; then
      echo "이 설치 파일은 Apple 실리콘(M1·M2 등) Mac 전용이라 이 인텔 Mac에서는 설치할 수 없습니다 — www.cysinsight.com 에서 인텔용 설치 파일(이름에 x64가 들어간 .dmg)을 받아 다시 설치해 주세요." >&2
      return 5
    fi
    return 0
  fi
  [ "$os" != "-" ] || return 0
  pymin="$(ic_minos "$py" "${run##*:}")"
  if [ -n "$pymin" ] && ic_ver_lt "$os" "$pymin"; then
    nodemin=""
    [ -n "$node" ] && [ -f "$node" ] && nodemin="$(ic_minos_of "$node" "${run%%:*}")"
    if [ -n "$nodemin" ] && ic_ver_lt "$pymin" "$nodemin"; then
      echo "이 Mac의 macOS 버전($os)이 낮아 cys를 설치할 수 없습니다 — 동봉된 Python은 macOS ${pymin%.0} 이상, 제품의 일부(동봉된 node 등)는 macOS ${nodemin%.0} 이상이 필요하니 macOS를 업데이트한 뒤 다시 설치해 주세요." >&2
    else
      echo "이 Mac의 macOS 버전($os)이 낮아 cys를 설치할 수 없습니다 — 동봉된 Python은 macOS ${pymin%.0} 이상이 필요하니 macOS를 업데이트한 뒤 다시 설치해 주세요." >&2
    fi
    return 5
  fi
  return 0
}
# <<< T11-PY-RUNNABLE

# ── 실행 인터프리터 선택 (진위 검증을 통과한 소스의 동봉 python 우선) ──
SRC_PY="$SRC/Contents/Resources/runtime/python/bin/python3"
if [ -x "$SRC_PY" ]; then
  PY="$SRC_PY"
else
  PY="/usr/bin/python3"   # 폴백(소스 동봉 python 결손 시) — CLT 미설치 맥에선 실패할 수 있음(finding 7)
fi
# T11 · BQ1(d) · CA3: 동봉 python 을 고른 경우에만, exec 전에 이 Mac 에서 돌 수 있는지 본다(위 블록 · 읽기 전용).
#   칩은 sysctl hw.optional.arm64(Rosetta 아래에서도 실제 칩), Rosetta 2 는 필요할 때만 시스템 바이너리를 실제로 번역해
#   본다(/usr/bin/arch -x86_64 /usr/bin/true). 기계 사실을 읽지 못하면 막지 않는다 — **확실한 판정**에서만 멈춘다.
if [ "$PY" = "$SRC_PY" ]; then
  read -r IC_HW IC_HWSUB IC_OS <<IC_EOF
$(ic_machine)
IC_EOF
  ic_python_runnable "$PY" "$IC_HW" "$IC_HWSUB" "$IC_OS" probe "$SRC/Contents/Resources/runtime/node/bin/node" || exit 5
fi

# 정본 ATOMIC-1 집행부에 그대로 위임(exec — 종료 코드 그대로 전파).
# PYTHONDONTWRITEBYTECODE=1: 읽기 전용 DMG 소스/설치 위치에 .pyc 를 쓰려는 시도 자체를 차단(봉인 보호).
export PYTHONDONTWRITEBYTECODE=1
exec "$PY" "$WRAPPER" install "$SRC" "$DEST"
