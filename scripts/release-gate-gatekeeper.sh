#!/usr/bin/env bash
# release-gate-gatekeeper.sh — **CI 릴리스 게이트**: 공증·스테이플까지 끝난 산출물이
# 실제 사용자 환경(브라우저 다운로드 = com.apple.quarantine 부착)에서 Gatekeeper 를
# 통과하는지 업로드 **전에** 기계로 평가한다. 봉인 파손·공증 누락 산출물의 발행을 차단한다.
#
# ★왜 quarantine 을 붙이는가
#   quarantine 이 없으면 Gatekeeper 의 **전체 재검증 경로 자체가 돌지 않는다**. curl 사본만
#   보던 종전 검증이 2026-08-01 사고("손상되었기 때문에 열 수 없습니다")를 한 번도 재현하지
#   못한 이유가 그것이다. 여기서는 실측 형식의 quarantine 을 부착한 뒤 평가한다.
#
# 검사 (대상 앱마다)
#   ① quarantine 부착·상속   — DMG 에 부착 → 마운트 → ditto 복사본이 quarantine 을 상속했는지
#   ② codesign --verify --deep --strict --verbose=2  — 봉인 무결(파손·추가 파일 검출)
#   ③ xcrun stapler validate — 공증 티켓 동봉(오프라인 증거)
#   ④ spctl --assess --type execute --verbose=4 — Gatekeeper 실평가 (accepted 필수)
#   ⑤ SEAL-2 불변식 정적 검사 — 동봉 python 런타임 트리(.py N개)가 opt-0/1/2 3레벨 .pyc 로
#      **전량 선컴파일**(개수 == 3N · 레벨별 == N)돼 있고, 표본 .pyc 헤더 flags==1
#      (unchecked-hash)인지. 불변식 정의처 = scripts/precompile-bundled-python.sh.
#      하나라도 어긋나면 그 레벨/파일로 부르는 순간 CPython 이 번들 안에 .pyc 를 새로 써서
#      봉인이 깨진다(2026-08-01 실사고 재발 경로). ★실행 0 — 파일 계수(find)와 헤더 8바이트
#      판독(러너 python3)만 한다. **번들 안 python 실행 절대 금지**: .app 안 바이너리를 한 번
#      이라도 exec 하면 macOS 가 앱 번들 보호를 걸어 SIGKILL·거짓 PASS 를 만든다
#      (docs/RELEASE.md 의 ⑥ 확장자 분리 실측 — 같은 함정).
#
# ★스코프 판정(공백 B · Windows 레인 · 2026-08-20): Windows 산출물에는 SEAL-2 선컴파일이 없고
#   이 게이트도 macOS 전용이다 — 여기서 수리하지 않는다. 오너 앵커: 윈도우 설치파일은 신중 접근,
#   그리고 코드서명 봉인 파손→Gatekeeper 차단은 macOS 고유 경로라 Windows 는 피해 경로가 아니라는 판정.
#
# ★spctl 정책 강등 (무음 통과 금지)
#   러너·머신 정책에 따라 `spctl --status` 가 "assessments disabled" 일 수 있다. 그때 ④ 는
#   **skip 이 아니라** 게이트를 "codesign 검증 단독 모드(DEGRADED)"로 **강등**하고 그 사실을
#   큰 배너로 로그에 남긴다. 강등 모드에서도 ①②③ 은 전부 필수이며 하나라도 실패하면 exit 1 이다.
#   (③ stapler validate 가 강등 모드의 공증 증거를 대신 잡는다 — 없으면 "서명은 됐지만 공증
#    안 된 앱"이 조용히 통과하는 구멍이 생긴다.)
#
# 대상 앱 = DMG 안의 **모든** *.app (maxdepth 2)
#   이 제품의 DMG 레이아웃(scripts/build-macos-signed.sh:282-293)은 최상위에 설치 도우미
#   `Install cys.app`, 숨김 `.support/cys.app` 에 진짜 앱이다. 사용자는 설치 도우미를 먼저
#   실행하므로 **둘 다** 평가해야 한다. 최상위 *.app 만 보면 진짜 앱의 봉인 파손을 놓친다.
#
# 사용
#   bash scripts/release-gate-gatekeeper.sh <DMG경로>
#   bash scripts/release-gate-gatekeeper.sh /Applications/cys.app      # .app 직접 지정(로컬 스모크)
#   옵션: --keep(작업 폴더 보존) · --quarantine-value <문자열>(기본 = 실측 Safari 형식)
#
# 종료 코드
#   0 = 전 검사 PASS (강등 모드면 ④ 제외 전부 PASS · 배너로 명시)
#   1 = 하나 이상 FAIL → **업로드 금지**
#   2 = 사용법 오류 · 도구 부재 · 마운트/복사 실패 · ⑤ 계수/표본 판독 불능 = **판정 불가(통과가 아니다)**
#   판정에 도달한 실행(exit 0·1)은 stdout 마지막 줄에 기계 요약 `GATE_MODE=full|degraded` 를
#   고정 출력한다 — CI(release.yml 게이트 스텝)가 GITHUB_STEP_SUMMARY 로 승격하는 증적 라인이다.
#
# scripts/verify-gatekeeper-user-path.sh 와의 관계 — 겹치지 않는 상·하위 게이트다.
#   그쪽(로컬·수동)은 여기 검사 전부 + ⑥ **봉인 자기파괴 재현**(동봉 python 을 실제로 스폰)까지
#   본다. 대신 대상 아키텍처 python 을 실행하므로 **arm64 러너에서 x64 DMG 를 보려면 Rosetta 2 가
#   필요**하고(없으면 FAIL) 임시 디스크 ~2GB 를 쓴다 — 그대로 CI 매트릭스에 걸면 x64 레그가
#   구조적으로 깨진다. 이 스크립트는 실행을 전혀 하지 않는 **정적 평가만** 남겨 양 레그에서
#   결정론으로 도는 CI 판(判)이다. 로컬 릴리스 절차에서는 여전히 그쪽을 돌려라(docs/RELEASE.md).
set -uo pipefail

KEEP=0
QVAL=""
TARGET=""

usage() {
  cat <<'USAGE'
사용: bash scripts/release-gate-gatekeeper.sh [옵션] <DMG경로 | .app경로>
옵션:
  --keep                    작업 폴더를 지우지 않는다(디버깅)
  --quarantine-value <str>  부착할 com.apple.quarantine 값(기본: <flags>;<epoch16>;CI;<UUID>)
  -h, --help                이 도움말
종료: 0=PASS · 1=FAIL(업로드 금지) · 2=판정 불가
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --keep) KEEP=1; shift ;;
    --quarantine-value) QVAL="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "알 수 없는 옵션: $1" >&2; usage >&2; exit 2 ;;
    *) TARGET="$1"; shift ;;
  esac
done

[ -n "$TARGET" ] || { usage >&2; exit 2; }

# ── 도구 fail-closed (없으면 판정 불가 = exit 2 · 통과 아님) ──
for t in hdiutil spctl codesign xattr ditto find uuidgen; do
  command -v "$t" >/dev/null 2>&1 || { echo "✗ 필수 도구 없음: $t (macOS + Xcode CLT 필요)" >&2; exit 2; }
done
command -v xcrun >/dev/null 2>&1 || { echo "✗ xcrun 없음 — Xcode CLT 필요(stapler validate 불가)" >&2; exit 2; }

# ── ⑤용 러너 python3 해소 (fail-closed · 번들 인터프리터 절대 금지) ──
# 헤더 8바이트(magic 4 + flags 4) 파싱은 인터프리터 버전 무관 — 러너 python3 로 충분하다.
# 단, cys pane 은 동봉 runtime 을 PATH 선두에 물리므로 `command -v python3` 가 **.app 안**
# python 으로 해소될 수 있다. .app 안 exec 는 위 ⑤ 머리 주석의 함정(앱 번들 보호·SIGKILL·
# 거짓 PASS) 그 자체다 — 물리 경로에 ".app/" 가 들어간 후보는 전부 기각한다.
GATE_PY=""
for cand in /usr/bin/python3 "$(command -v python3 2>/dev/null || true)"; do
  [ -n "$cand" ] && [ -x "$cand" ] || continue
  real="$(readlink -f -- "$cand" 2>/dev/null || printf '%s' "$cand")"
  case "$cand:$real" in *".app/"*) continue ;; esac
  GATE_PY="$cand"; break
done
[ -n "$GATE_PY" ] || { echo "✗ 번들 밖 python3 없음 — ⑤ SEAL-2 표본 판독 불가(측정 불능은 통과가 아니다)" >&2; exit 2; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/cys-gk-gate.XXXXXX")" || exit 2
# macOS 는 /var·/tmp 가 심볼릭링크라 mount·codesign 이 실경로(/private/…)로 되돌려 출력한다.
# 여기서 정규화하지 않으면 mount 출력 대조와 경로 치환이 빗나간다(verify-gatekeeper-user-path.sh 실측).
WORK="$(cd "$WORK" && pwd -P)"
MOUNTS=()
cleanup() {
  local m
  for m in "${MOUNTS[@]:-}"; do
    [ -n "$m" ] || continue
    hdiutil detach "$m" -force >/dev/null 2>&1 || hdiutil detach "$m" -force >/dev/null 2>&1
  done
  if [ "$KEEP" = "1" ]; then echo "작업 폴더 보존: $WORK"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT INT TERM

PASS_N=0
FAIL_N=0
ok()   { PASS_N=$((PASS_N+1)); printf 'PASS %s%s\n' "$1" "${2:+ | $2}"; }
bad()  { FAIL_N=$((FAIL_N+1)); printf 'FAIL %s%s\n' "$1" "${2:+ | $2}"; }
info() { printf '     %s\n' "$1"; }

# ── spctl 정책 선확인 → 모드 결정 (무음 통과 금지) ──
SPCTL_STATUS="$(spctl --status 2>&1 || true)"
if printf '%s' "$SPCTL_STATUS" | grep -qi 'assessments enabled'; then
  MODE="full"
else
  MODE="degraded"
fi

echo "═══ macOS 릴리스 게이트: 격리 속성 + Gatekeeper 실평가 ═══"
echo "대상 : $TARGET"
echo "작업 : $WORK"
echo "spctl: $SPCTL_STATUS → 모드=$MODE"
if [ "$MODE" = "degraded" ]; then
  cat <<'BANNER'
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
!!  ★게이트 강등 — spctl 평가 불능 (assessments disabled)
!!  이 실행은 Gatekeeper **실평가(spctl --assess)를 수행하지 못했다**.
!!  codesign 봉인 검증 + stapler 공증 티켓 검증 **단독 모드**로 내려간다.
!!  skip 이 아니다: ①②③ 은 그대로 필수이며 실패 시 exit 1 이다.
!!  그러나 "Gatekeeper 통과"는 이 실행으로 증명되지 않았다 — 판정 범위가 좁아졌다.
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
BANNER
  echo "::warning title=Gatekeeper 게이트 강등::spctl assessments disabled — codesign/stapler 단독 모드로 평가함(실평가 미수행)"
fi
echo

# ── ⑤ SEAL-2 불변식 정적 검사 (실행 0 — find 계수 + 러너 python3 헤더 판독만) ──
# 불변식(정의처 scripts/precompile-bundled-python.sh): 동봉 런타임 트리의 모든 .py(N개)는
# 서명 **전에** opt-0/1/2 3레벨 .pyc 로 선컴파일돼 봉인에 들어 있고(총 3N · 레벨별 N),
# 전량 unchecked-hash(헤더 flags==1 · PEP 552: bit0=hash-based, bit1=check_source)다.
# 검사 범위는 runtime **트리 전체**(python stdlib + node 동봉 gyp) — precompile 스크립트의
# 컴파일 범위와 정확히 같다(python/lib 만 보면 gyp 갭이 남는다).
# ★magic 4바이트는 버전마다 달라 러너 python 의 MAGIC_NUMBER 와 대조하지 않는다(러너≠번들
#   버전이면 오탐). flags 필드 오프셋(4..8)·리틀엔디언은 전 버전 동일 — 인터프리터 무관 판독.
# 반환: 0=검사 수행(위반은 ok/bad 로 집계 — FAIL 은 말미 판정이 exit 1) · 2=판정 불가 · 3=대상 아님(python 런타임 미동봉 앱)
seal2_static_check() {
  local app="$1" name rt n_py n_opt0 n_opt1 n_opt2 n_all s_out s_rc
  name="$(basename "$app")"
  rt="$app/Contents/Resources/runtime"
  [ -d "$rt/python/lib" ] || return 3
  # N = __pycache__ 밖 .py 전수 — precompile 검증(PYCHECK)과 동일 산식.
  n_py=$(find "$rt" -type d -name __pycache__ -prune -o -type f -name '*.py' -print 2>/dev/null | wc -l | tr -d ' ')
  n_opt0=$(find "$rt" -type f -path '*/__pycache__/*.pyc' ! -name '*.opt-1.pyc' ! -name '*.opt-2.pyc' 2>/dev/null | wc -l | tr -d ' ')
  n_opt1=$(find "$rt" -type f -path '*/__pycache__/*.opt-1.pyc' 2>/dev/null | wc -l | tr -d ' ')
  n_opt2=$(find "$rt" -type f -path '*/__pycache__/*.opt-2.pyc' 2>/dev/null | wc -l | tr -d ' ')
  n_all=$(find "$rt" -type f -path '*/__pycache__/*.pyc' 2>/dev/null | wc -l | tr -d ' ')
  if [ "$n_py" -eq 0 ]; then
    echo "✗ ⑤ SEAL-2($name): python 런타임 디렉터리는 있는데 .py 가 0개 — 계수 불성립(레이아웃 변경?)" >&2
    return 2
  fi
  info "⑤ 계수($name): .py $n_py · __pycache__/*.pyc $n_all (opt-0 $n_opt0 · opt-1 $n_opt1 · opt-2 $n_opt2) · 기대 3N=$((n_py*3))"
  if [ "$n_opt0" -eq "$n_py" ] && [ "$n_opt1" -eq "$n_py" ] && [ "$n_opt2" -eq "$n_py" ] && [ "$n_all" -eq $((n_py*3)) ]; then
    ok "⑤ SEAL-2 선컴파일 커버리지($name)" "3레벨 × $n_py = $n_all"
  else
    bad "⑤ SEAL-2 선컴파일 커버리지($name)" \
        "레벨 결손/과잉(.py $n_py 대비 opt-0 $n_opt0 · opt-1 $n_opt1 · opt-2 $n_opt2 · 총 $n_all ≠ 3N) — 결손 레벨로 부르는 순간 번들에 .pyc 가 쓰여 봉인이 깨진다"
  fi
  # 표본 헤더 flags==1 — 러너 python3($GATE_PY). 판독 실패는 위반이 아니라 판정 불가(rc 2).
  s_out="$("$GATE_PY" - "$rt" <<'PYSEAL'
import os, struct, sys
root = sys.argv[1]
pycs = []
for dp, _dn, fn in os.walk(root):
    if os.path.basename(dp) != "__pycache__":
        continue
    for f in fn:
        if f.endswith(".pyc"):
            pycs.append(os.path.join(dp, f))
pycs.sort()
if not pycs:
    print("NO-PYC"); sys.exit(2)
step = max(1, len(pycs) // 24)
samples = pycs[::step][:25]
bad = []
for p in samples:
    try:
        with open(p, "rb") as fh:
            head = fh.read(8)
    except OSError as e:
        print("READ-FAIL %s: %s" % (os.path.relpath(p, root), e)); sys.exit(2)
    if len(head) < 8:
        print("SHORT-HEADER %s" % os.path.relpath(p, root)); sys.exit(2)
    flags = struct.unpack("<I", head[4:8])[0]
    if flags != 1:
        kind = "timestamp" if not (flags & 1) else ("checked-hash" if flags == 3 else "flags=%d" % flags)
        bad.append("%s: %s(flags=%d)" % (os.path.relpath(p, root), kind, flags))
if bad:
    print("BAD %d/%d" % (len(bad), len(samples)))
    for b in bad[:10]:
        print("  - " + b)
    sys.exit(1)
print("표본 %d/%d 전부 flags=1(unchecked-hash)" % (len(samples), len(pycs)))
PYSEAL
)"; s_rc=$?
  case "$s_rc" in
    0) ok "⑤ SEAL-2 표본 헤더($name)" "$s_out" ;;
    1) bad "⑤ SEAL-2 표본 헤더($name)" "unchecked-hash 아님 — 재컴파일·재기록이 살아난다: $(printf '%s' "$s_out" | tr '\n' ' ')" ;;
    *) echo "✗ ⑤ SEAL-2($name) 표본 판독 불가(rc=$s_rc): $(printf '%s' "$s_out" | tr '\n' ' ')" >&2
       return 2 ;;
  esac
  return 0
}
SEAL2_TARGETS=0   # python 런타임을 실제로 검사한 앱 수 — 0이면 판정 불가(측정 불능≠통과)

# ── quarantine 값 (실측 4필드 형식: <flags4hex>;<epoch16진>;<에이전트>;<UUID>) ──
# 실측 표본: 0281;6a62b3d6;Chrome;1772ABA8-…  ·  0083;6a62000f;Safari;137E3038-…
[ -n "$QVAL" ] || QVAL="0083;$(printf '%x' "$(date +%s)");CI;$(uuidgen)"

APPS=()          # 평가 대상 .app 원본 경로
SRC_KIND=""

case "$TARGET" in
  *.dmg)
    [ -f "$TARGET" ] || { echo "✗ DMG 없음: $TARGET" >&2; exit 2; }
    SRC_KIND="dmg"
    # ★원본 산출물을 건드리지 않는다 — 업로드될 바이트에 xattr 를 쓰지 않기 위해 사본에 부착한다.
    DMG="$WORK/$(basename "$TARGET")"
    cp "$TARGET" "$DMG" || { echo "✗ DMG 복사 실패" >&2; exit 2; }
    info "DMG $(basename "$DMG") · $(wc -c <"$DMG" | tr -d ' ') bytes (원본 무변경 · 사본에 부착)"

    if xattr -w com.apple.quarantine "$QVAL" "$DMG" 2>/dev/null &&
       [ "$(xattr -p com.apple.quarantine "$DMG" 2>/dev/null)" = "$QVAL" ]; then
      ok "① quarantine 부착(DMG)" "$QVAL"
    else
      bad "① quarantine 부착(DMG)" "xattr -w 실패 — 이후 검사는 실사용자 경로가 아니다"
      echo; echo "=== 판정 불가 ==="; exit 2
    fi

    MP="$WORK/mnt"
    mkdir -p "$MP"
    if hdiutil attach "$DMG" -mountpoint "$MP" -nobrowse -readonly -noverify -noautoopen >"$WORK/attach.log" 2>&1; then
      MOUNTS+=("$MP")
      QMOUNT=$(mount | grep -F " $MP " | grep -c quarantine || true)
      if [ "${QMOUNT:-0}" -ge 1 ]; then
        ok "① 마운트(격리 볼륨)" "$MP"
      else
        bad "① 마운트(격리 볼륨)" "quarantine 플래그 없이 마운트됨 — 사용자 경로 모사 실패"
      fi
    else
      bad "① 마운트" "$(tail -3 "$WORK/attach.log" | tr '\n' ' ')"
      echo; echo "=== 판정 불가 ==="; exit 2
    fi

    # ★모든 *.app (maxdepth 2) — 설치 도우미(최상위) + 숨김 .support/cys.app 둘 다.
    while IFS= read -r a; do [ -n "$a" ] && APPS+=("$a"); done \
      < <(find "$MP" -maxdepth 2 -name '*.app' -type d -print 2>/dev/null | sort)
    if [ "${#APPS[@]}" -eq 0 ]; then
      bad "① 앱 번들 탐색" "$MP 에서 *.app 없음(maxdepth 2)"
      echo; echo "=== 판정 불가 ==="; exit 2
    fi
    info "발견한 앱 번들 ${#APPS[@]}개: $(for a in "${APPS[@]}"; do printf '%s ' "${a#"$MP"/}"; done)"
    ;;
  *.app|*.app/)
    TARGET="${TARGET%/}"
    [ -d "$TARGET" ] || { echo "✗ .app 없음: $TARGET" >&2; exit 2; }
    SRC_KIND="app"
    APPS+=("$TARGET")
    info ".app 직접 지정 모드 — DMG 마운트 없이 사본에 quarantine 을 직접 부착한다"
    ;;
  *)
    echo "✗ 대상은 .dmg 파일 또는 .app 디렉터리여야 한다: $TARGET" >&2; exit 2 ;;
esac
echo

INSTALL_DIR="$WORK/Applications"
mkdir -p "$INSTALL_DIR"

for APP_SRC in "${APPS[@]}"; do
  APP_NAME="$(basename "$APP_SRC")"
  APP="$INSTALL_DIR/$APP_NAME"
  echo "── 대상 앱: $APP_NAME ──"

  # ditto = Finder 드래그 복사와 동일하게 확장속성(quarantine 포함)을 보존한다.
  if ! ditto "$APP_SRC" "$APP" >"$WORK/ditto.log" 2>&1; then
    bad "① 설치 모사 복사($APP_NAME)" "$(tail -3 "$WORK/ditto.log" | tr '\n' ' ')"
    echo; echo "=== 판정 불가 ==="; exit 2
  fi

  if [ "$SRC_KIND" = "app" ]; then
    # 다운로드 사본을 모사 — 번들 루트에 부착(= 격리 DMG 에서 복사했을 때 실측되는 상태와 동일)
    xattr -w com.apple.quarantine "$QVAL" "$APP" 2>/dev/null || true
  fi
  QAPP="$(xattr -p com.apple.quarantine "$APP" 2>/dev/null || true)"
  if [ -n "$QAPP" ]; then
    ok "① quarantine $( [ "$SRC_KIND" = "app" ] && echo 부착 || echo 상속 )($APP_NAME)" "$QAPP"
  else
    bad "① quarantine $( [ "$SRC_KIND" = "app" ] && echo 부착 || echo 상속 )($APP_NAME)" \
        "복사본에 quarantine 없음 — Gatekeeper 전체 재검증 경로가 안 돈다(검증 무효)"
  fi

  # ── ② 봉인 무결 ──
  CS_OUT="$(codesign --verify --deep --strict --verbose=2 "$APP" 2>&1)"; CS_RC=$?
  if [ "$CS_RC" -eq 0 ]; then
    ok "② codesign --verify --deep --strict($APP_NAME)" "valid on disk · satisfies its Designated Requirement"
  else
    bad "② codesign --verify --deep --strict($APP_NAME)" "rc=$CS_RC"
    echo "     ── codesign 출력(verbatim) ──"
    printf '%s\n' "$CS_OUT" | sed "s|$APP|<app>|g" | sed 's/^/     | /'
  fi

  # ── ③ 공증 티켓 동봉 (강등 모드의 유일한 공증 증거) ──
  ST_OUT="$(xcrun stapler validate "$APP" 2>&1)"; ST_RC=$?
  if [ "$ST_RC" -eq 0 ]; then
    ok "③ stapler validate($APP_NAME)" "공증 티켓 동봉 확인"
  else
    bad "③ stapler validate($APP_NAME)" "rc=$ST_RC · $(printf '%s' "$ST_OUT" | tail -2 | tr '\n' ' ')"
  fi

  # ── ④ Gatekeeper 실평가 (full 모드에서만 · degraded 는 위 배너로 고지) ──
  if [ "$MODE" = "full" ]; then
    SPCTL_OUT="$(spctl --assess --type execute --verbose=4 "$APP" 2>&1)"; SPCTL_RC=$?
    if [ "$SPCTL_RC" -eq 0 ] && printf '%s' "$SPCTL_OUT" | grep -q "accepted"; then
      ok "④ spctl --assess --type execute($APP_NAME)" "$(printf '%s' "$SPCTL_OUT" | tr '\n' ' ' | sed "s|$APP|<app>|g")"
    else
      bad "④ spctl --assess --type execute($APP_NAME)" "rc=$SPCTL_RC"
      echo "     ── spctl 출력(verbatim) ──"
      printf '%s\n' "$SPCTL_OUT" | sed "s|$APP|<app>|g" | sed 's/^/     | /'
    fi
  else
    echo "SKIP ④ spctl --assess($APP_NAME) — assessments disabled (게이트 강등 · 위 배너 참조)"
  fi

  # ── ⑤ SEAL-2 불변식 정적 검사 — 원본 트리(마운트된 DMG 안 / 직접 지정 .app) 판독 전용 ──
  seal2_static_check "$APP_SRC"; SEAL2_RC=$?
  case "$SEAL2_RC" in
    0) SEAL2_TARGETS=$((SEAL2_TARGETS+1)) ;;
    3) info "⑤ SEAL-2($APP_NAME): python 런타임 미동봉 — 대상 아님(설치 도우미 등)" ;;
    *) echo; echo "=== 판정 불가 ==="; exit 2 ;;
  esac

  # ── 실패 시 진단: 서명 요약 ──
  if [ "$CS_RC" -ne 0 ] || { [ "$MODE" = "full" ] && [ "${SPCTL_RC:-1}" -ne 0 ]; } || [ "$ST_RC" -ne 0 ]; then
    echo "     ── codesign -dv --verbose=4 요약 ──"
    codesign -dv --verbose=4 "$APP" 2>&1 | sed "s|$APP|<app>|g" | sed 's/^/     | /'
  fi
  echo
done

# ⑤ 를 한 앱에서도 못 돌렸다면(진짜 앱에 python 런타임이 안 보임) 측정이 없었던 것이다 —
# 측정 불능은 통과가 아니다. 레이아웃이 정말 바뀌었다면 이 게이트를 의도적으로 갱신하라.
if [ "$SEAL2_TARGETS" -eq 0 ]; then
  echo "✗ ⑤ SEAL-2: 평가한 앱 어디에도 동봉 python 런타임(Contents/Resources/runtime/python/lib)이 없다" >&2
  echo; echo "=== 판정 불가 ==="; exit 2
fi

echo "═══ 판정 ═══"
echo "모드: $MODE $( [ "$MODE" = "degraded" ] && echo '(spctl 실평가 미수행 — 강등)' )"
echo "PASS=$PASS_N · FAIL=$FAIL_N"
RC=0
if [ "$FAIL_N" -gt 0 ]; then
  echo "✗ Gatekeeper 게이트 FAIL — 이 산출물은 업로드·발행 금지"
  echo "::error title=Gatekeeper 게이트 FAIL::$TARGET — 격리 상태 평가에서 $FAIL_N 건 실패(위 verbatim 출력 참조)"
  RC=1
elif [ "$MODE" = "degraded" ]; then
  echo "✓ 강등 모드 전 항목 PASS — 단, Gatekeeper 실평가는 수행되지 않았다(무음 통과 아님·배너 고지됨)"
else
  echo "✓ 전 항목 PASS — 격리된 사본이 Gatekeeper 실평가를 통과했다"
fi
# 기계 요약(마지막 줄 고정 · 헤더 종료 코드 항 참조) — CI 가 GITHUB_STEP_SUMMARY 로 승격한다.
echo "GATE_MODE=$MODE"
exit "$RC"
