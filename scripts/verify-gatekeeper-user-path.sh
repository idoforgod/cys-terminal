#!/usr/bin/env bash
# verify-gatekeeper-user-path.sh — 실사용자 경로(브라우저 다운로드 → 드래그 설치 → 첫 실행)를
# 그대로 재현하는 **릴리스 게이트**. 2026-08-01 실사고("손상되었기 때문에 열 수 없습니다")의
# 검증 구멍을 닫는다.
#
# ★왜 필요한가 (2026-08-01 근본원인)
#   앱 번들의 동봉 Python 이 실행 중 `Contents/Resources/runtime/python/lib/python3.12/
#   **/__pycache__/*.pyc` 를 **번들 안에** 새로 쓴다 → 코드서명 봉인이 스스로 깨진다
#   (`codesign --verify` = "a sealed resource is missing or invalid / file added: …
#   __pycache__/_compression.cpython-312.pyc"). 공증·staple 자체는 정상이다.
#   사용자가 **브라우저로 받으면** 파일에 `com.apple.quarantine` 이 붙고, 첫 실행 시
#   Gatekeeper 가 **전체 재검증**을 돌리면서 깨진 봉인을 잡아내 실행을 차단한다.
#
#   검증 구멍: 릴리스 ⓓ 검증은 `curl` 사본(=quarantine 없음)만 봤다. quarantine 이 없으면
#   Gatekeeper 전체 재검증 경로 자체가 안 돌아서, 이 사고 경로를 **한 번도 재현하지 못했다**.
#   이 스크립트가 그 경로를 기계로 재현한다.
#
# 검사 6단계 (전부 통과해야 exit 0)
#   ① quarantine 부착 — 실제 브라우저 다운로드에서 실측한 형식으로 DMG 에 부착
#   ② 마운트 — 격리 볼륨으로 붙는지 확인
#   ③ 드래그 설치 모사 — `ditto` 로 앱을 임시 폴더에 복사하고 **quarantine 상속**을 확인
#      (상속이 없으면 그 뒤 검사가 전부 무의미해지므로 여기서 FAIL 한다)
#   ④ Gatekeeper 평가 — `spctl --assess --type execute --verbose=4` = accepted
#   ⑤ 서명·공증 정합 — `codesign --verify --deep --strict` + `stapler validate`
#   ⑥ ★봉인 자기파괴 재현 — 동봉 python 을 한 번 돌린 뒤 `codesign --verify` 재실행.
#      "file added / file modified / sealed resource" 가 나오면 **FAIL**(이번 사고의 정확한 재현).
#
# ★⑥ 이 `.app` 확장자를 뗀 사본(probe)에서 도는 이유 — 실측 근거 (2026-08-01, macOS 25.5.0)
#   macOS 는 `.app` 번들에서 바이너리를 한 번이라도 exec 하면 그 번들을 **앱 번들 보호**로
#   잠근다. 그 뒤 셸이 띄운 python 의 `.pyc` 쓰기는 EPERM 으로 막혀 **결함이 있어도 아무 일도
#   안 일어난 것처럼 보인다**(측정: `.app` 사본에서 python 실행 → .pyc 3→3, codesign exit 0
#   = 거짓 PASS). 실제 사고 경로에서는 쓰는 주체가 **앱 자신**(같은 Team ID)이라 이 보호를
#   통과해 쓰기가 성사된다 — 그래서 `/Applications/cys.app` 은 실제로 깨졌다.
#   `.app` 확장자만 뗀 동일 사본에서는 보호가 걸리지 않아 결함이 그대로 드러난다
#   (측정: .pyc 3→30, `codesign --verify` = "file added …/re/__pycache__/_parser…pyc", exit 1).
#   즉 확장자를 떼는 것은 **검사 대상을 약화시키는 우회가 아니라, 검증기 머신에서만 발생하는
#   OS 보호가 결함을 가리는 것을 걷어내는 조치**다. 서명·봉인 내용은 완전히 동일하다.
#
# ⑥-A(plain env) 와 ⑥-B(PYTHONDONTWRITEBYTECODE=1) 를 모두 돌린다
#   B = SEAL-1 완화(앱이 python 을 스폰할 때 얹는 env)가 실제로 쓰기를 막는지.
#   A = 그 env 를 못 타는 경로(셸·훅·pane 에서 도는 python)에서도 번들이 안 깨지는지
#       = **패키징 층위 불변식**. 서명 전 `compileall` 로 stdlib .pyc 를 봉인에 넣거나
#       런타임을 봉인 밖으로 빼야 A 가 통과한다. A 는 완화가 아니라 구조로만 닫힌다.
#   ★A·B 모두 **동봉 python 의 종료코드(rc)≠0 이면 그 자리에서 FAIL** 한다(2026-08-01 추가).
#     실행이 죽으면 ".pyc 개수 불변 + codesign 통과"가 저절로 성립해 버리므로, rc 를 보지 않는
#     검사는 **측정 불능을 통과로 세는 구멍**이 된다(실측 사례: 격리된 미공증 사본의 nested
#     python = AMFI 가 즉시 SIGKILL, exit 137 — 아무것도 안 돌았는데 두 술어는 다 참이 된다).
#
# 사용
#   bash scripts/verify-gatekeeper-user-path.sh <DMG경로>
#   bash scripts/verify-gatekeeper-user-path.sh https://www.cysinsight.com/downloads/cys_0.14.9_aarch64.dmg
#   bash scripts/verify-gatekeeper-user-path.sh --version 0.14.9 [--arch aarch64|x64]
#   옵션: --keep(작업 폴더 보존) · --quarantine-flags <4자리hex>(기본 = ~/Downloads 실측값)
#
# 종료 코드
#   0 = 전 단계 PASS (실사용자 경로 안전)
#   1 = 하나 이상 FAIL (배포 금지)
#   2 = 사용법 오류 · 도구 부재 · 자산 획득 실패 (**판정 불가 — 통과가 아니다**)
#
# 전제·비용 (실측 v0.14.9)
#   · macOS 에서만 의미가 있다(hdiutil·spctl·codesign·xattr·xcrun). Xcode CLT 필요.
#   · 임시 작업 폴더에 **DMG 227MB + 앱 사본 492MB × 3 ≒ 2GB** 를 쓴다. 끝나면 지운다(--keep 로 보존).
#   · 소요 ~1분(로컬 DMG) / ~1분30초(원격 다운로드 포함).
#   · 대상 아키텍처의 python 을 실제로 실행하므로, arm64 머신에서 x64 DMG 를 볼 땐 Rosetta 2 가
#     필요하다. 없으면 ⑥-A·⑥-B 가 "동봉 python 실행 실패 — 검사 불성립"으로 **FAIL** 한다(측정 불능은
#     통과가 아니다). 확실히 하려면 각 아키텍처 머신에서 자기 DMG 를 돌려라.
#   · 앱을 **실행하지 않는다**(동봉 python 만 1회 스폰). 데몬·창이 뜨지 않는다.
#
# 기존 `scripts/verify-release-remote.py` 와의 관계: 그건 **발행 후** 홈페이지 표기·링크·
# 해시가 맞는지(=올바른 파일이 올라갔는지)를 본다. 이건 **그 파일을 실제로 받아 설치했을 때
# 열리는지**를 본다. 겹치지 않는 직교 게이트다 — 둘 다 돌려야 한다.
set -uo pipefail

SITE_DL="https://www.cysinsight.com/downloads"
KEEP=0
QFLAGS=""
DMG_SRC=""
VERSION=""
ARCH=""

# 파일 머리 주석(2행~`set -` 직전)을 그대로 도움말로 쓴다 — 문서와 코드가 갈라지지 않게.
usage() { awk 'NR>1 && /^set -/{exit} NR>1{sub(/^# ?/,""); print}' "$0"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --keep) KEEP=1; shift ;;
    --quarantine-flags) QFLAGS="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --arch) ARCH="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "알 수 없는 옵션: $1" >&2; exit 2 ;;
    *) DMG_SRC="$1"; shift ;;
  esac
done

# ── 도구 fail-closed (없으면 판정 불가 = exit 2) ──
for t in hdiutil spctl codesign xattr ditto uuidgen find; do
  command -v "$t" >/dev/null 2>&1 || { echo "✗ 필수 도구 없음: $t" >&2; exit 2; }
done
command -v xcrun >/dev/null 2>&1 || { echo "✗ Xcode CLT 필요(xcrun) — xcode-select --install" >&2; exit 2; }

# ── 대상 DMG 결정 (로컬 경로 · 원격 URL · 버전/아키텍처) ──
if [ -z "$DMG_SRC" ]; then
  [ -n "$VERSION" ] || { usage >&2; exit 2; }
  case "${ARCH:-}" in
    "") ARCH=$([ "$(uname -m)" = "arm64" ] && echo aarch64 || echo x64) ;;
    aarch64|x64) ;;
    *) echo "✗ --arch 는 aarch64|x64" >&2; exit 2 ;;
  esac
  DMG_SRC="$SITE_DL/cys_${VERSION}_${ARCH}.dmg"
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/gk-user-path.XXXXXX")" || exit 2
# ★ macOS 는 /var·/tmp 가 심볼릭링크라 `mount`·`codesign` 이 경로를 **실경로**(/private/…)로
#   되돌려 출력한다. 여기서 한 번 정규화해 두지 않으면 (a) 격리 플래그 대조가 항상 빗나가고
#   (b) 출력에서 임시경로를 지우는 치환이 절반만 먹는다(실측).
WORK="$(cd "$WORK" && pwd -P)"
MOUNTS=()
cleanup() {
  local m
  for m in "${MOUNTS[@]:-}"; do
    [ -n "$m" ] || continue
    hdiutil detach "$m" -force >/dev/null 2>&1 || hdiutil detach "$m" -force >/dev/null 2>&1
  done
  if [ "$KEEP" = "1" ]; then
    echo "작업 폴더 보존: $WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT INT TERM

PASS_N=0
FAIL_N=0
ok()   { PASS_N=$((PASS_N+1)); printf 'PASS %s%s\n' "$1" "${2:+ | $2}"; }
bad()  { FAIL_N=$((FAIL_N+1)); printf 'FAIL %s%s\n' "$1" "${2:+ | $2}"; }
info() { printf '     %s\n' "$1"; }

echo "═══ Gatekeeper 실사용자 경로 검증 ═══"
echo "대상: $DMG_SRC"
echo "작업: $WORK"
echo

# ── 자산 확보 ──
DMG="$WORK/$(basename "${DMG_SRC%%\?*}")"
case "$DMG_SRC" in
  http://*|https://*)
    echo "── 원격 자산 다운로드 (브라우저 다운로드와 동일한 바이트) ──"
    if ! curl -fL --progress-bar --max-time 1800 -A "Mozilla/5.0" -o "$DMG" "$DMG_SRC"; then
      echo "✗ 다운로드 실패: $DMG_SRC" >&2; exit 2
    fi
    ;;
  *)
    [ -f "$DMG_SRC" ] || { echo "✗ DMG 없음: $DMG_SRC" >&2; exit 2; }
    cp "$DMG_SRC" "$DMG" || exit 2
    ;;
esac
[ -s "$DMG" ] || { echo "✗ DMG 가 비었다: $DMG" >&2; exit 2; }
info "DMG $(basename "$DMG") · $(wc -c <"$DMG" | tr -d ' ') bytes"
echo

# ── ① quarantine 부착 (형식은 실제 브라우저 다운로드에서 실측) ──
# 실측 형식(4필드 세미콜론 구분): <flags4hex>;<epoch16진>;<다운로드 에이전트>;<UUID>
#   0281;6a62b3d6;Chrome;1772ABA8-…   0083;6a62000f;Safari;137E3038-…
# flags 는 브라우저·상태에 따라 다르므로 이 머신의 실물 다운로드에서 그대로 뽑아 쓴다
# (없으면 0081 = 다운로드 비트만 선 표준값).
detect_qflags() {
  local f v
  while IFS= read -r f; do
    v="$(xattr -p com.apple.quarantine "$f" 2>/dev/null | head -1)"
    case "$v" in
      ????\;*) printf '%s' "${v%%;*}"; return 0 ;;
    esac
  done < <(find "$HOME/Downloads" -maxdepth 1 -type f \( -name '*.dmg' -o -name '*.pkg' -o -name '*.zip' \) 2>/dev/null | head -20)
  return 1
}
if [ -z "$QFLAGS" ]; then
  QFLAGS="$(detect_qflags || true)"
  if [ -n "$QFLAGS" ]; then QSRC="~/Downloads 실측"; else QFLAGS="0081"; QSRC="기본값(실측 표본 없음)"; fi
else
  QSRC="인자 지정"
fi
QVAL="$QFLAGS;$(printf '%x' "$(date +%s)");Chrome;$(uuidgen)"
if xattr -w com.apple.quarantine "$QVAL" "$DMG" 2>/dev/null &&
   [ "$(xattr -p com.apple.quarantine "$DMG" 2>/dev/null)" = "$QVAL" ]; then
  ok "① quarantine 부착" "$QVAL ($QSRC)"
else
  bad "① quarantine 부착" "xattr -w 실패 — 이후 검사는 실사용자 경로가 아니다"
  echo; echo "=== 판정 불가 ==="; exit 2
fi

# ── ② 마운트 ──
MP="$WORK/mnt"   # $WORK 는 위에서 실경로로 정규화됨 — mount 출력과 그대로 대조된다
mkdir -p "$MP"
if hdiutil attach "$DMG" -mountpoint "$MP" -nobrowse -readonly -noverify -noautoopen >"$WORK/attach.log" 2>&1; then
  MOUNTS+=("$MP")
  QMOUNT=$(mount | grep -F " $MP " | grep -c quarantine || true)
  if [ "${QMOUNT:-0}" -ge 1 ]; then
    ok "② 마운트(격리 볼륨)" "$MP"
  else
    bad "② 마운트(격리 볼륨)" "quarantine 플래그 없이 마운트됨 — 사용자 경로 모사 실패"
  fi
else
  bad "② 마운트" "$(tail -3 "$WORK/attach.log" | tr '\n' ' ')"
  echo; echo "=== 판정 불가 ==="; exit 2
fi

# ★검사 대상은 사용자가 실제로 실행하는 cys.app 이다 — 설치 도우미 DMG 레이아웃에서 최상위 유일
#   *.app 은 'Install cys.app'(설치 도우미 애플릿)이고 진짜 cys.app 은 .support/cys.app(depth 2)로
#   숨겨진다. 최상위 *.app 만 잡으면(구 로직) ⑥ 봉인 자기파괴 재현이 동봉 python 없는 설치 도우미를
#   보고 vacuous PASS 한다(finding 5). 그래서 cys.app 을 이름으로 우선 탐색(depth 2 = .support 포함)하고,
#   없을 때만 최상위 *.app 으로 폴백(설치 도우미 없는 구 레이아웃 DMG 호환).
APP_SRC="$(find "$MP" -maxdepth 2 -name 'cys.app' -type d -print -quit 2>/dev/null)"
[ -n "$APP_SRC" ] || APP_SRC="$(find "$MP" -maxdepth 1 -name '*.app' -print -quit 2>/dev/null)"
if [ -z "$APP_SRC" ]; then
  bad "② 앱 번들 탐색" "$MP 에서 cys.app(및 최상위 *.app) 없음"
  echo; echo "=== 판정 불가 ==="; exit 2
fi
APP_NAME="$(basename "$APP_SRC")"
info "앱 번들: $APP_NAME"
echo

# ── ③ 드래그 설치 모사 (ditto = Finder 복사와 동일하게 확장속성 보존) ──
INSTALL_DIR="$WORK/Applications"
APP="$INSTALL_DIR/$APP_NAME"
mkdir -p "$INSTALL_DIR"
if ! ditto "$APP_SRC" "$APP" >"$WORK/ditto.log" 2>&1; then
  bad "③ 드래그 설치 모사" "$(tail -3 "$WORK/ditto.log" | tr '\n' ' ')"
  echo; echo "=== 판정 불가 ==="; exit 2
fi
QAPP="$(xattr -p com.apple.quarantine "$APP" 2>/dev/null || true)"
if [ -n "$QAPP" ]; then
  ok "③ 드래그 설치 모사 + quarantine 상속" "$QAPP"
else
  bad "③ 드래그 설치 모사 + quarantine 상속" "복사본에 quarantine 없음 — Gatekeeper 전체 재검증 경로가 안 돈다(검증 무효)"
fi

# ── ④ Gatekeeper 평가 ──
SPCTL_OUT="$(spctl --assess --type execute --verbose=4 "$APP" 2>&1)"
SPCTL_RC=$?
if [ "$SPCTL_RC" -eq 0 ] && printf '%s' "$SPCTL_OUT" | grep -q "accepted"; then
  ok "④ spctl --assess --type execute" "$(printf '%s' "$SPCTL_OUT" | tr '\n' ' ' | sed "s|$APP|<app>|g")"
else
  bad "④ spctl --assess --type execute" "rc=$SPCTL_RC · $(printf '%s' "$SPCTL_OUT" | tr '\n' ' ' | sed "s|$APP|<app>|g")"
fi

# ── ⑤ 서명·공증 정합 ──
CS_OUT="$(codesign --verify --deep --strict --verbose=2 "$APP" 2>&1)"
CS_RC=$?
if [ "$CS_RC" -eq 0 ]; then
  ok "⑤-a codesign --verify --deep --strict" "valid on disk · DR 충족"
else
  bad "⑤-a codesign --verify --deep --strict" "rc=$CS_RC · $(printf '%s' "$CS_OUT" | grep -Ei 'sealed|file (added|modified|missing)' | head -3 | tr '\n' ' ' | sed "s|$APP|<app>|g")"
fi
ST_OUT="$(xcrun stapler validate "$APP" 2>&1)"
ST_RC=$?
if [ "$ST_RC" -eq 0 ]; then
  ok "⑤-b stapler validate (공증 티켓 동봉)" "worked"
else
  bad "⑤-b stapler validate (공증 티켓 동봉)" "rc=$ST_RC · $(printf '%s' "$ST_OUT" | tail -2 | tr '\n' ' ')"
fi
echo

# ── ⑥ ★봉인 자기파괴 재현 ──
# probe = 동일 사본에서 `.app` 확장자만 뗀 것(파일 앞부분 주석의 실측 근거 참조).
# ★A·B 는 **각각 새 사본**에서 돈다 — 한 사본에서 연달아 돌리면 앞선 실행이 남긴 상태(.pyc·
#   OS 보호 래치)가 뒤 검사의 판정을 오염시킨다. 판정을 지는 A 는 반드시 무오염 사본에서.
mkdir -p "$WORK/probe"
make_probe() {  # $1 = 태그 → 표준출력으로 경로
  local p="$WORK/probe/${APP_NAME%.app}-$1"
  cp -R "$APP_SRC" "$p" >"$WORK/probe.$1.log" 2>&1 || return 1
  printf '%s' "$p"
}
# ★탐색 술어(2026-08-02 수정): 228b930 이 python3 을 python3.12 심링크로 복원 → `-type f` 가
#   인터프리터를 놓쳐 ⑥ 이 측정 불능=FAIL 로 접혔다. 심링크도 후보로 잡되, 해석 후 정규파일이며
#   실행 가능([ -f ]·[ -x ] 는 링크를 따라간다)일 때만 채택 — 끊긴 링크는 여전히 미발견=FAIL
#   (fail-closed 보존). ⑥ 판정식·PYC 계수·codesign 호출은 건드리지 않았다.
find_py() { local p; p="$(find "$1/Contents" \( -type f -o -type l \) -perm +111 -name 'python3' -print -quit 2>/dev/null)"; [ -f "$p" ] && [ -x "$p" ] && printf '%s' "$p"; }

PROBE_A="$(make_probe a || true)"
if [ -z "$PROBE_A" ] || [ ! -d "$PROBE_A" ]; then
  bad "⑥ 봉인 자기파괴 재현" "probe 사본 생성 실패: $(tail -2 "$WORK/probe.a.log" 2>/dev/null | tr '\n' ' ')"
else
  # ⑥-0 기준선 — 여기서 이미 깨져 있으면 DMG 가 깨진 봉인을 실어 보낸 것이다.
  if codesign --verify --deep --strict "$PROBE_A" >"$WORK/cs0.log" 2>&1; then
    ok "⑥-0 probe 기준선(실행 전 봉인)" "clean"
  else
    bad "⑥-0 probe 기준선(실행 전 봉인)" "DMG 가 이미 깨진 봉인을 싣고 있다: $(grep -Ei 'sealed|file (added|modified)' "$WORK/cs0.log" | head -2 | tr '\n' ' ' | sed "s|$PROBE_A|<probe>|g")"
  fi

  PY="$(find_py "$PROBE_A")"
  if [ -z "$PY" ]; then
    if [ -d "$PROBE_A/Contents/Resources/runtime" ]; then
      bad "⑥ 봉인 자기파괴 재현" "runtime/ 은 있는데 동봉 python3 을 못 찾음 — 경로 표류(판정 불가는 통과가 아니다)"
    else
      ok "⑥ 봉인 자기파괴 재현" "동봉 인터프리터 없음 — 이 결함군 해당 없음"
    fi
  else
    info "동봉 python: ${PY#"$PROBE_A"/}"
    PYC0=$(find "$PROBE_A" -name '*.pyc' 2>/dev/null | wc -l | tr -d ' ')

    # ⑥-A ★본 검사(무오염 사본): env 완화를 못 타는 경로에서도 번들이 안 깨져야 한다
    #      = 패키징 층위 불변식. 사고의 정확한 재현 검사다.
    "$PY" -c "import argparse,tempfile,uuid" >"$WORK/pyA.log" 2>&1
    PY_RC=$?
    PYC_A=$(find "$PROBE_A" -name '*.pyc' 2>/dev/null | wc -l | tr -d ' ')
    if [ "$PY_RC" -ne 0 ]; then
      bad "⑥-A ★봉인 자기파괴 재현" "동봉 python 실행 실패(rc=$PY_RC) — 검사 불성립: $(tail -2 "$WORK/pyA.log" | tr '\n' ' ')"
    else
      # --verbose=2 라야 "file added: …/__pycache__/*.pyc" 개별 줄까지 나온다(요약만으로는
      # 어느 파일이 봉인을 깼는지 안 보인다 — 사고 진단 원문과 같은 형태로 남기려면 필요).
      codesign --verify --deep --strict --verbose=2 "$PROBE_A" >"$WORK/csA.log" 2>&1
      CSA_RC=$?
      BREAK="$(grep -Eic 'a sealed resource is missing or invalid|file added|file modified' "$WORK/csA.log" || true)"
      if [ "$CSA_RC" -eq 0 ] && [ "${BREAK:-0}" -eq 0 ] && [ "$PYC_A" = "$PYC0" ]; then
        ok "⑥-A ★봉인 자기파괴 재현" ".pyc ${PYC0}→$PYC_A · 실행 후에도 봉인 유지"
      else
        bad "⑥-A ★봉인 자기파괴 재현" "동봉 python 1회 실행으로 봉인 파손 · .pyc ${PYC0}→$PYC_A · codesign rc=$CSA_RC"
        grep -Ei 'a sealed resource is missing or invalid|file added|file modified' "$WORK/csA.log" 2>/dev/null |
          head -5 | sed "s|$PROBE_A|<probe>|g" | while IFS= read -r l; do info "↳ $l"; done
        info "↳ 이것이 2026-08-01 사고의 정확한 재현이다 — 실사용자 머신에서는 앱이 자기 번들에"
        info "  .pyc 를 쓰는 순간 봉인이 깨지고, quarantine 이 붙은 사본은 Gatekeeper 전체"
        info "  재검증에서 '손상되었기 때문에 열 수 없습니다'로 차단된다."
      fi
    fi

    # ⑥-B (새 사본): SEAL-1 완화(앱이 python 스폰 시 얹는 env)가 실제로 쓰기를 막는지.
    PROBE_B="$(make_probe b || true)"
    if [ -z "$PROBE_B" ] || [ ! -d "$PROBE_B" ]; then
      bad "⑥-B SEAL-1 완화 확인" "probe 사본 생성 실패: $(tail -2 "$WORK/probe.b.log" 2>/dev/null | tr '\n' ' ')"
    else
      PYB="$(find_py "$PROBE_B")"
      PYCB0=$(find "$PROBE_B" -name '*.pyc' 2>/dev/null | wc -l | tr -d ' ')
      PYTHONDONTWRITEBYTECODE=1 "$PYB" -c "import argparse,tempfile,uuid" >"$WORK/pyB.log" 2>&1
      PYB_RC=$?
      PYC_B=$(find "$PROBE_B" -name '*.pyc' 2>/dev/null | wc -l | tr -d ' ')
      # ★rc 하드페일 — ⑥-A(PY_RC) 와 동일한 계약. python 이 아예 못 돌면 "완화가 쓰기를 막았다"가
      #   아니라 **아무 일도 일어나지 않은 것**인데, 그때도 `.pyc 개수 불변`과 `codesign 통과`는
      #   저절로 참이 되어 PASS 가 새어 나간다(측정 불능 ≠ 통과). 아래 두 술어는 그대로 두고
      #   rc 조건만 앞에 얹는다 — 통과 조건이 좁아지기만 하고 넓어지지 않는다.
      if [ "$PYB_RC" -ne 0 ]; then
        bad "⑥-B SEAL-1 완화(PYTHONDONTWRITEBYTECODE=1)" "동봉 python 실행 실패(rc=$PYB_RC) — 검사 불성립(측정 불능은 통과가 아니다) · .pyc ${PYCB0}→$PYC_B: $(tail -2 "$WORK/pyB.log" 2>/dev/null | tr '\n' ' ')"
      elif codesign --verify --deep --strict --verbose=2 "$PROBE_B" >"$WORK/csB.log" 2>&1 && [ "$PYC_B" = "$PYCB0" ]; then
        ok "⑥-B SEAL-1 완화(PYTHONDONTWRITEBYTECODE=1)" ".pyc ${PYCB0}→$PYC_B · 봉인 유지"
      else
        bad "⑥-B SEAL-1 완화(PYTHONDONTWRITEBYTECODE=1)" "완화가 무효다 · .pyc ${PYCB0}→$PYC_B · $(grep -Ei 'sealed|file (added|modified)' "$WORK/csB.log" | head -2 | tr '\n' ' ' | sed "s|$PROBE_B|<probe>|g")"
      fi
    fi
  fi
fi

echo
echo "═══ $PASS_N PASS / $FAIL_N FAIL ═══"
if [ "$FAIL_N" -eq 0 ]; then
  echo "판정: PASS — 브라우저 다운로드 → 드래그 설치 → 첫 실행 경로가 안전하다."
  exit 0
fi
echo "판정: FAIL — 이 빌드는 실사용자 경로에서 차단될 수 있다. 배포 금지."
exit 1
