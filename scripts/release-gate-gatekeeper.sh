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
#   ⑤ SEAL-2 불변식 전칭(∀) 정적 검사 — 동봉 python 런타임 트리의 **모든** .py 각각에
#      기대 .pyc 3종(__pycache__/<stem>.<tag>{,.opt-1,.opt-2}.pyc · <tag> 는 하드코딩 없이
#      실재 pyc 파일명에서 추출)이 실재하고(파일별 대응 — 총계 상쇄 불가), 소스 없는 고아
#      pyc 가 0이며, 발견된 pyc **전량**의 헤더 flags==1(unchecked-hash)인지(표본화 제거 —
#      F1 전칭 격상 2026-08-20). 불변식 정의처 = scripts/precompile-bundled-python.sh.
#      하나라도 어긋나면 그 레벨/파일로 부르는 순간 CPython 이 번들 안에 .pyc 를 새로 써서
#      봉인이 깨진다(2026-08-01 실사고 재발 경로). ★실행 0 — 디렉터리 워크·이름 대조와 헤더
#      8바이트 판독(러너 python3 단일 호출)만 한다. **번들 안 python 실행 절대 금지**: .app 안 바이너리를 한 번
#      이라도 exec 하면 macOS 가 앱 번들 보호를 걸어 SIGKILL·거짓 PASS 를 만든다
#      (docs/RELEASE.md 의 ⑥ 확장자 분리 실측 — 같은 함정).
#   ⑥ 첫-부팅 기록자 모사(설치 후 상태) — W5-4 신설(2026-08-29 · W3b/W3c 회귀 그물의 CI 판).
#      2026-08-28 실사고에서 봉인을 깬 것은 산출물이 아니라 **제품 자신의 첫-부팅 기록자**였다:
#      팩 preflight C11b 가 페인 PATH 선두(=번들 Contents/MacOS)를 따라 번들 안에 cys-dept
#      심링크를 만들었다. ② 는 "빌드 시점 봉인"만 증명하고 이 계급을 못 본다. 여기서는 설치
#      모사 사본에 그 기록자를 **러너 python3 로 실제 실행**(번들 python 스폰은 여전히 0)한 뒤
#      ⓐ번들 파일 전수 센서스 diff == 0(예상치 못한 파일 생성 = 이름 지목 FAIL)
#      ⓑcodesign --verify --deep --strict 재통과 를 단언한다. HOME·PATH·팩 env 는 임시
#      스크래치로 격리(하네스 test_preflight_c11b_seal.py 와 동형) — 실환경 무접촉.
#
# ★스코프 판정(공백 B · Windows 레인 · 2026-08-20): Windows 산출물에는 SEAL-2 선컴파일이 없고
#   이 게이트도 macOS 전용이다 — 여기서 수리하지 않는다. 오너 앵커: 윈도우 설치파일은 신중 접근,
#   그리고 코드서명 봉인 파손→Gatekeeper 차단은 macOS 고유 경로라 Windows 는 피해 경로가 아니라는 판정.
#
# ★spctl 정책 강등 = 판정 불가 폐쇄 (F2 · 2026-08-20 — 측정 불능≠통과)
#   러너·머신 정책에 따라 `spctl --status` 가 "assessments disabled" 일 수 있다. 종전에는
#   ④ 만 생략하는 "codesign/stapler 단독 모드(DEGRADED)"로 강등해 최종 rc=0 을 냈으나,
#   발행 승인 경로(release.yml 게이트 스텝 · release-postprocess.py 5단계)가 rc=0 을 승인
#   신호로 소비하므로 그것은 "측정 불능≠통과" 계약 위반이다. 기본(릴리스) 모드에서
#   degraded 는 이제 **`GATE_MODE=degraded` 1줄 출력 후 exit 2(판정 불가)로 폐쇄**된다.
#   강등 평가가 필요한 진단은 옵트인 `--diagnose-degraded-ok`(LOUD 고지 · 발행 경로 사용
#   금지 — release.yml·release-postprocess.py 가 이 플래그를 싣지 않음은
#   scripts/tests/test_release_postprocess_gate.py 가 문자열 핀으로 못박는다)로만 연다.
#   CI 실측(v0.14.19 · run 32039644404)은 macOS 양 레그 모두 `assessments enabled → 모드
#   =full` 이었으므로 이 폐쇄로 인한 릴리스 경로 회귀는 0이다.
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
#         --diagnose-degraded-ok(진단 전용 — degraded 폐쇄를 열어 ①②③⑤⑥ 강등 평가 · 발행 경로 사용 금지)
#         --seal2-only(진단 전용 — 대상 .app 에 ⑤ 전칭 검사만 단독 실행 · 적대 픽스처 테스트의 호출 지점)
#
# 종료 코드
#   0 = 전 검사 PASS (기본 모드에선 full 에서만 도달 가능 · --diagnose-degraded-ok 의 0 은 진단용이다)
#   1 = 하나 이상 FAIL → **업로드 금지**
#   2 = 사용법 오류 · 도구 부재 · 마운트/복사 실패 · ⑤ 판독 불능 · **degraded 폐쇄(기본 모드
#       · F2)** = **판정 불가(통과가 아니다)**
#   GATE_MODE 계약: 판정에 도달한 실행(exit 0·1)은 stdout 마지막 줄에 기계 요약
#   `GATE_MODE=full|degraded` 를 고정 출력하고, **degraded-폐쇄 exit 2 도 폐쇄 직전에
#   `GATE_MODE=degraded` 를 출력**한다(그 외 exit 2 경로는 미출력) — CI(release.yml 게이트
#   스텝)가 GITHUB_STEP_SUMMARY 로 승격하는 증적 라인이다.
#
# scripts/verify-gatekeeper-user-path.sh 와의 관계 — 겹치지 않는 상·하위 게이트다.
#   그쪽(로컬·수동)은 여기 검사에 더해 **봉인 자기파괴 재현**(동봉 python 을 실제로 스폰)까지
#   본다. 대신 대상 아키텍처 python 을 실행하므로 **arm64 러너에서 x64 DMG 를 보려면 Rosetta 2 가
#   필요**하고(없으면 FAIL) 임시 디스크 ~2GB 를 쓴다 — 그대로 CI 매트릭스에 걸면 x64 레그가
#   구조적으로 깨진다. 이 스크립트는 실행을 전혀 하지 않는 **정적 평가만** 남겨 양 레그에서
#   결정론으로 도는 CI 판(判)이다. 로컬 릴리스 절차에서는 여전히 그쪽을 돌려라(docs/RELEASE.md).
set -uo pipefail

KEEP=0
QVAL=""
TARGET=""
DIAG_DEGRADED=0
SEAL2_ONLY=0

usage() {
  cat <<'USAGE'
사용: bash scripts/release-gate-gatekeeper.sh [옵션] <DMG경로 | .app경로>
옵션:
  --keep                    작업 폴더를 지우지 않는다(디버깅)
  --quarantine-value <str>  부착할 com.apple.quarantine 값(기본: <flags>;<epoch16>;CI;<UUID>)
  --diagnose-degraded-ok    진단 전용: degraded(spctl assessments disabled) 폐쇄를 열어
                            ①②③⑤⑥ 강등 평가를 돈다 — **발행 경로 사용 금지**(테스트 핀)
  --seal2-only              진단 전용: 대상 .app 에 ⑤ SEAL-2 전칭 검사만 단독 실행
  -h, --help                이 도움말
종료: 0=PASS · 1=FAIL(업로드 금지) · 2=판정 불가(degraded 폐쇄 포함)
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --keep) KEEP=1; shift ;;
    --quarantine-value) QVAL="${2:-}"; shift 2 ;;
    --diagnose-degraded-ok) DIAG_DEGRADED=1; shift ;;
    --seal2-only) SEAL2_ONLY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) echo "알 수 없는 옵션: $1" >&2; usage >&2; exit 2 ;;
    *) TARGET="$1"; shift ;;
  esac
done

[ -n "$TARGET" ] || { usage >&2; exit 2; }

# ── 도구 fail-closed (없으면 판정 불가 = exit 2 · 통과 아님) ──
# --seal2-only(진단)는 ⑤ 만 돌므로 러너 python3 해소만 요구한다 — macOS 밖(픽스처 테스트)에서도 돈다.
if [ "$SEAL2_ONLY" != "1" ]; then
  for t in hdiutil spctl codesign xattr ditto find uuidgen; do
    command -v "$t" >/dev/null 2>&1 || { echo "✗ 필수 도구 없음: $t (macOS + Xcode CLT 필요)" >&2; exit 2; }
  done
  command -v xcrun >/dev/null 2>&1 || { echo "✗ xcrun 없음 — Xcode CLT 필요(stapler validate 불가)" >&2; exit 2; }
fi

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
[ -n "$GATE_PY" ] || { echo "✗ 번들 밖 python3 없음 — ⑤ SEAL-2 전칭 판독 불가(측정 불능은 통과가 아니다)" >&2; exit 2; }

# ── ⑥용 팩 preflight 해소(리포 상대) — 첫-부팅 기록자 모사의 실행 대상 스크립트 ──
# 이 게이트는 리포 체크아웃에서 돈다(release.yml 게이트 스텝·release-postprocess.py — 실측 소비자
# 전부). 부재는 skip 이 아니라 판정 불가다(⑥ 이 못 돌면 첫-부팅 계급이 무검증 — 함수 안에서 폐쇄).
GATE_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
PREFLIGHT_PY="$GATE_SCRIPT_DIR/../cysjavis-pack/bin/javis_preflight.py"

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

# ── ⑤ SEAL-2 불변식 전칭(∀) 정적 검사 (실행 0 — 러너 python3 단일 호출 1회 · 전량 판독) ──
# 불변식(정의처 scripts/precompile-bundled-python.sh): 동봉 런타임 트리의 모든 .py 는
# 서명 **전에** opt-0/1/2 3레벨 .pyc 로 선컴파일돼 봉인에 들어 있고, 전량 unchecked-hash
# (헤더 flags==1 · PEP 552: bit0=hash-based, bit1=check_source)다.
# 검사 범위는 runtime **트리 전체**(python stdlib + node 동봉 gyp) — precompile 스크립트의
# 컴파일 범위와 정확히 같다(python/lib 만 보면 gyp 갭이 남는다).
# ★F1 전칭 격상(2026-08-20): 종전 "레벨별 총계 동일성 + 표본 25개 flags"는 (a)결손 1 +
#   동수 고아 1 의 총계 상쇄와 (b)표본 밖 flags 변조를 통과시켰다(둘 다 적대 픽스처로
#   test_release_postprocess_gate.py Seal2UniversalCheckTests 에 박제 — FAIL 재현 강제).
#   지금은 단일 python 호출 1회로 ①파일별 3레벨 대응(<tag> 하드코딩 금지 — 실재 pyc
#   파일명에서 추출) ②고아 pyc 0(기대 집합 밖 pyc 전부 = 소스 없는 pyc·이탈 태그 포함)
#   ③발견된 pyc 전량 헤더 8바이트 flags==1 을 검사한다.
#   성능 실측(2026-08-20 · M-시리즈 로컬): .py 1,140 · pyc 3,420 전량 헤더 판독이
#   /Applications/cys.app 트리 0.10초 · v0.14.19 DMG(aarch64) 마운트 트리 0.13초 —
#   표본화가 필요 없는 비용이다.
# ★magic 4바이트는 버전마다 달라 러너 python 의 MAGIC_NUMBER 와 대조하지 않는다(러너≠번들
#   버전이면 오탐). flags 필드 오프셋(4..8)·리틀엔디언은 전 버전 동일 — 인터프리터 무관 판독.
# 반환: 0=검사 수행(위반은 ok/bad 로 집계 — FAIL 은 말미 판정이 exit 1) · 2=판정 불가 · 3=대상 아님(python 런타임 미동봉 앱)
seal2_static_check() {
  local app="$1" name rt s_out s_rc
  name="$(basename "$app")"
  rt="$app/Contents/Resources/runtime"
  [ -d "$rt/python/lib" ] || return 3
  s_out="$("$GATE_PY" - "$rt" <<'PYSEAL'
import os, struct, sys, time
root = sys.argv[1]
t0 = time.monotonic()
CAP = 10  # 위반 상세 출력 상한(종류별) — 전체 건수는 FAIL 요약 줄이 든다
py_files = []   # (디렉터리, stem)
pycs = []
for dp, dn, fn in os.walk(root):
    if os.path.basename(dp) == "__pycache__":
        dn[:] = []
        for f in fn:
            if f.endswith(".pyc"):
                pycs.append(os.path.join(dp, f))
        continue
    for f in fn:
        if f.endswith(".py"):
            py_files.append((dp, f[:-3]))
if not py_files:
    print("NO-PY: python 런타임 디렉터리는 있는데 .py 가 0개 — 계수 불성립(레이아웃 변경?)")
    sys.exit(2)
if not pycs:
    # 측정은 성립했다(.py N개 · pyc 0개) — 판정 불가가 아니라 전량 결손 위반이다.
    print("FAIL: .py %d개인데 pyc 0개 — 선컴파일 전량 결손" % len(py_files))
    sys.exit(1)

# <tag> 추출 — 하드코딩 금지: 실재 pyc 이름 <stem>.<tag>[.opt-N].pyc 에서 뽑는다.
tag_count = {}
for p in pycs:
    s = os.path.basename(p)[:-4]
    for o in (".opt-1", ".opt-2"):
        if s.endswith(o):
            s = s[:-len(o)]
            break
    if "." in s:
        t = s.rsplit(".", 1)[1]
        tag_count[t] = tag_count.get(t, 0) + 1
if not tag_count:
    print("FAIL: pyc %d개 전부 무태그 이름 — 선컴파일 산출물 형식이 아니다" % len(pycs))
    sys.exit(1)
tag = max(tag_count, key=tag_count.get)   # 지배 태그 — 이탈 태그 pyc 는 아래 ② 고아로 걸린다

# ① 파일별 대응 — .py 마다 기대 pyc 3종 실재(레벨별 총계 동일성의 상쇄 허용 결함 제거)
expected = set()
missing = []
for dp, stem in py_files:
    for opt in ("", ".opt-1", ".opt-2"):
        e = os.path.join(dp, "__pycache__", "%s.%s%s.pyc" % (stem, tag, opt))
        expected.add(e)
        if not os.path.isfile(e):
            missing.append(os.path.relpath(e, root))
# ② 고아 — 기대 집합(실재 .py × 3레벨 × 지배 태그) 밖의 모든 pyc:
#    소스 없는 pyc·이탈 태그·무태그 pyc 가 전부 여기 걸린다(총계 상쇄 불가).
orphans = [os.path.relpath(p, root) for p in pycs if p not in expected]
# ③ flags 전수 — 발견된 pyc 전량 헤더 8바이트(magic 4 + flags 4) 판독(표본화 제거)
badflags = []
for p in pycs:
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
        badflags.append("%s: %s(flags=%d)" % (os.path.relpath(p, root), kind, flags))
dt = time.monotonic() - t0
print("계수: .py %d · pyc %d · tag=%s · 전수 판독 %.2fs" % (len(py_files), len(pycs), tag, dt))
nv = 0
for label, items in (("MISSING(기대 pyc 결손)", missing),
                     ("ORPHAN(고아 pyc)", orphans),
                     ("BADFLAGS(flags!=1)", badflags)):
    if items:
        nv += len(items)
        print("%s %d건 — 상한 %d건만 출력:" % (label, len(items), CAP))
        for it in items[:CAP]:
            print("  - " + it)
if nv:
    print("FAIL: missing=%d · orphan=%d · badflags=%d — 결손 레벨/파일로 부르는 순간 CPython 이 번들 안에 .pyc 를 새로 써 봉인이 깨진다"
          % (len(missing), len(orphans), len(badflags)))
    sys.exit(1)
print("OK: .py %d × 3레벨 전수 대응 · 고아 0 · flags==1 전수 %d/%d · 판독 %.2fs"
      % (len(py_files), len(pycs), len(pycs), dt))
PYSEAL
)"; s_rc=$?
  case "$s_rc" in
    0) printf '%s\n' "$s_out" | sed 's/^/     ⑤ /'
       ok "⑤ SEAL-2 전칭 검사($name)" "$(printf '%s\n' "$s_out" | tail -1)" ;;
    1) printf '%s\n' "$s_out" | sed 's/^/     ⑤ /'
       bad "⑤ SEAL-2 전칭 검사($name)" "$(printf '%s\n' "$s_out" | tail -1)" ;;
    *) echo "✗ ⑤ SEAL-2($name) 판독 불능(rc=$s_rc): $(printf '%s' "$s_out" | tr '\n' ' ')" >&2
       return 2 ;;
  esac
  return 0
}
SEAL2_TARGETS=0   # python 런타임을 실제로 검사한 앱 수 — 0이면 판정 불가(측정 불능≠통과)

# ── 봉인 위반 파일 이름 지목(②·⑥ 재검증 실패 시) — codesign 출력에서 file/resource added·
#    modified·missing 라인만 뽑아 기계 라인(SEAL_CULPRIT:)으로 승격한다. verbatim 덤프는 사람용,
#    이 라인은 요약·감사가 긁는 판독원이다(2026-08-28 실사고의 culprit 도 'file added' 1줄이었다).
print_seal_culprits() {
  local out="$1" app="$2" c
  c="$(printf '%s\n' "$out" | grep -E '(file|resource) (added|modified|missing)' | sed "s|$app|<app>|g" | head -20 || true)"
  [ -n "$c" ] || return 0
  echo "     ── 봉인 위반 파일(이름 지목) ──"
  printf '%s\n' "$c" | sed 's/^/     SEAL_CULPRIT: /'
}

# ── ⑥ 첫-부팅 기록자 모사(설치 후 상태) — W3b/W3c 회귀 그물의 CI 판 (헤더 ⑥ 참조) ──
# 설치 모사 사본($INSTALL_DIR 밑 ditto 본 — 쓰기 가능 = 기록이 실제로 일어날 수 있는 지반)에
# 팩 preflight C11b(--fix 상당)를 러너 python3 로 실행하고, 실행 전후 번들 전수 센서스 diff 0 과
# codesign --verify --deep --strict 재통과를 단언한다. ★읽기 전용 DMG 마운트(APP_SRC)가 아니라
# 사본이어야 한다 — 읽기 전용 지반에선 결함 코드도 쓰기에 실패해 검사가 공허해진다.
# 반환: 0=수행(위반은 ok/bad 집계) · 2=판정 불가 · 3=대상 아님(Contents/MacOS/cys 부재 — 설치 도우미 등)
firstboot_sim_check() {
  local app="$1" name pre post sim_out sim_rc simhome added removed reverify_out reverify_rc viol
  name="$(basename "$app")"
  [ -x "$app/Contents/MacOS/cys" ] || return 3
  if [ ! -f "$PREFLIGHT_PY" ]; then
    echo "✗ ⑥ 첫-부팅 모사($name): 팩 preflight 부재($PREFLIGHT_PY) — 리포 체크아웃에서 실행하라(측정 불능은 통과가 아니다)" >&2
    return 2
  fi
  pre="$WORK/census-pre.txt"; post="$WORK/census-post.txt"
  ( cd "$app" && find . \( -type f -o -type l \) -print | LC_ALL=C sort ) > "$pre" || return 2
  simhome="$WORK/firstboot-home"
  rm -rf "$simhome"
  mkdir -p "$simhome/.cys/pack/bin" || return 2
  printf '#!/bin/sh\nexit 0\n' > "$simhome/.cys/pack/bin/cys-dept"
  chmod 0755 "$simhome/.cys/pack/bin/cys-dept"
  # 실사고 형상 재현: HOME=스크래치 · PATH 선두=사본 Contents/MacOS · 팩 env=스크래치 팩.
  # 전부 파이썬 프로세스 내부 env 로만 존재한다(러너/사용자 실환경 무접촉 · 하네스와 동형).
  sim_out="$("$GATE_PY" - "$PREFLIGHT_PY" "$app" "$simhome" <<'PYSIM'
import importlib.util, os, sys
pre_path, app, simhome = sys.argv[1], sys.argv[2], sys.argv[3]
os.environ["HOME"] = simhome
os.environ["PATH"] = os.path.join(app, "Contents", "MacOS")
sys.path.insert(0, os.path.dirname(pre_path))   # 형제 모듈(javis_lock 등) 해소
spec = importlib.util.spec_from_file_location("javis_preflight", pre_path)
mod = importlib.util.module_from_spec(spec)
sys.modules["javis_preflight"] = mod
try:
    spec.loader.exec_module(mod)
except Exception as e:
    print("IMPORT-FAIL: %r" % (e,))
    sys.exit(2)
for k in getattr(mod, "PACK_DIR_ENV_KEYS", ()) or ("CYS_PACK_DIR",):
    os.environ.pop(k, None)
os.environ["CYS_PACK_DIR"] = os.path.join(simhome, ".cys", "pack")
try:
    pf = mod.Preflight(fix=True, skips=[])
    pf.c11b_cys_dept_path()
except Exception as e:
    print("RUN-FAIL: %r" % (e,))
    sys.exit(2)
rows = [r for r in pf.results if str(r.get("id", "")).startswith("C11b")]
if not rows:
    print("NO-RESULT: C11b 가 결과를 내지 않았다 — preflight 구조 변경?")
    sys.exit(2)
row = rows[-1]
print("C11b: %s - %s" % (row["status"], row["detail"]))
link = os.path.join(simhome, ".local", "bin", "cys-dept")
bundle_link = os.path.join(app, "Contents", "MacOS", "cys-dept")
rc = 0
if os.path.lexists(bundle_link):
    print("VIOLATION: 기록자가 번들 안에 썼다: Contents/MacOS/cys-dept — 봉인 파손 재발(2026-08-28 계급)")
    rc = 1
if row["status"] not in ("PASS", "FIXED"):
    print("VIOLATION: C11b 판정 %s (기대 PASS/FIXED) — 기록자 행동이 계약과 다르다" % row["status"])
    rc = 1
if not os.path.islink(link):
    print("VIOLATION: 표준 위치(~/.local/bin/cys-dept) 링크 미생성 — 기록 경로가 실행되지 않아 모사가 성립하지 않는다")
    rc = 1
if rc == 0:
    print("OK: 기록자는 번들 밖(%s)에만 썼다" % link)
sys.exit(rc)
PYSIM
)"; sim_rc=$?
  printf '%s\n' "$sim_out" | sed "s|$app|<app>|g" | sed 's/^/     ⑥ /'
  if [ "$sim_rc" -ge 2 ]; then
    echo "✗ ⑥ 첫-부팅 모사($name) 실행 불능(rc=$sim_rc)" >&2
    return 2
  fi
  ( cd "$app" && find . \( -type f -o -type l \) -print | LC_ALL=C sort ) > "$post" || return 2
  added="$(comm -13 "$pre" "$post")"
  removed="$(comm -23 "$pre" "$post")"
  reverify_out="$(codesign --verify --deep --strict --verbose=2 "$app" 2>&1)"; reverify_rc=$?
  viol=""
  [ "$sim_rc" -eq 0 ] || viol="기록자 위반(위 ⑥ 출력)"
  if [ -n "$added" ]; then
    viol="${viol:+$viol · }번들 안 신규 파일 $(printf '%s\n' "$added" | grep -c .)건"
    echo "     ── ⑥ 기록자 실행 후 번들 안에 '추가된' 파일(이름 지목) ──"
    printf '%s\n' "$added" | head -20 | sed 's/^/     SEAL_CULPRIT(added): /'
  fi
  if [ -n "$removed" ]; then
    viol="${viol:+$viol · }번들 파일 소거 $(printf '%s\n' "$removed" | grep -c .)건"
    echo "     ── ⑥ 기록자 실행 후 번들에서 '사라진' 파일(이름 지목) ──"
    printf '%s\n' "$removed" | head -20 | sed 's/^/     SEAL_CULPRIT(removed): /'
  fi
  if [ "$reverify_rc" -ne 0 ]; then
    viol="${viol:+$viol · }실행 후 codesign 재검증 실패(rc=$reverify_rc)"
    echo "     ── ⑥ 실행 후 codesign 출력(verbatim) ──"
    printf '%s\n' "$reverify_out" | sed "s|$app|<app>|g" | sed 's/^/     | /'
    print_seal_culprits "$reverify_out" "$app"
  fi
  if [ -n "$viol" ]; then
    bad "⑥ 첫-부팅 기록자 모사($name)" "$viol"
  else
    ok "⑥ 첫-부팅 기록자 모사($name)" "기록자 실행 후 센서스 diff 0 · 봉인 재검증 통과(기록은 번들 밖 ~/.local/bin 에만)"
  fi
  return 0
}
SIM_TARGETS=0     # 첫-부팅 기록자 모사를 실제로 돌린 앱 수 — 0이면 PASS 선언 불가(말미 폐쇄)

# ── 진단 전용: --seal2-only — ⑤ 전칭 검사만 단독 실행 (게이트 판정 아님 · GATE_MODE 미출력) ──
#   적대 픽스처 박제(test_release_postprocess_gate.py Seal2UniversalCheckTests)의 호출 지점.
#   발행 경로(release.yml·release-postprocess.py)가 이 플래그를 싣지 않음은 같은 테스트의 핀이 지킨다.
if [ "$SEAL2_ONLY" = "1" ]; then
  TARGET="${TARGET%/}"
  [ -d "$TARGET" ] || { echo "✗ --seal2-only 대상 없음(.app 디렉터리 필요): $TARGET" >&2; exit 2; }
  echo "[진단 전용] --seal2-only: $TARGET — ⑤ SEAL-2 전칭 검사만 돈다(발행 판정 아님)"
  seal2_static_check "$TARGET"; SEAL2_RC=$?
  case "$SEAL2_RC" in
    0) ;;
    3) echo "✗ --seal2-only: 동봉 python 런타임 없음(Contents/Resources/runtime/python/lib)" >&2; exit 2 ;;
    *) exit 2 ;;
  esac
  [ "$FAIL_N" -gt 0 ] && exit 1
  exit 0
fi

# ── spctl 정책 선확인 → 모드 결정 (무음 통과 금지) ──
# CYS_GATE_FORCE_DEGRADED=1 = 시험 주입점(degraded 폐쇄의 단위 재현 전용): degraded **방향
# 으로만** 강제할 수 있다 — full 을 강제하는 주입점은 우회 벡터라 만들지 않는다(fail-closed 단방향).
SPCTL_STATUS="$(spctl --status 2>&1 || true)"
if [ "${CYS_GATE_FORCE_DEGRADED:-0}" = "1" ]; then
  MODE="degraded"
  SPCTL_STATUS="$SPCTL_STATUS [CYS_GATE_FORCE_DEGRADED=1 — 시험 주입: 강제 degraded]"
elif printf '%s' "$SPCTL_STATUS" | grep -qi 'assessments enabled'; then
  MODE="full"
else
  MODE="degraded"
fi

echo "═══ macOS 릴리스 게이트: 격리 속성 + Gatekeeper 실평가 ═══"
echo "대상 : $TARGET"
echo "작업 : $WORK"
echo "spctl: $SPCTL_STATUS → 모드=$MODE"
if [ "$MODE" = "degraded" ]; then
  if [ "$DIAG_DEGRADED" != "1" ]; then
    # ── F2 폐쇄(기본 = 릴리스 모드): degraded = 판정 불가 — 측정 불능은 통과가 아니다 ──
    #   종전엔 ④ 만 생략하고 최종 rc=0 을 냈다. 발행 승인 경로(release.yml 게이트 스텝 ·
    #   release-postprocess.py 5단계)가 rc=0 을 승인 신호로 소비하므로, spctl 실평가가
    #   없는 실행은 여기서 즉시 폐쇄한다. 진단은 --diagnose-degraded-ok(발행 경로 사용 금지).
    echo "✗ degraded(spctl assessments disabled) — Gatekeeper 실평가 불능 = 판정 불가(폐쇄 · exit 2)" >&2
    echo "::error title=Gatekeeper 게이트 폐쇄(degraded)::spctl assessments disabled — 실평가 불능은 통과가 아니다(exit 2). 진단은 --diagnose-degraded-ok(발행 경로 사용 금지)"
    echo "GATE_MODE=degraded"
    exit 2
  fi
  echo '!!!! [진단 전용] --diagnose-degraded-ok — 이 실행의 exit 0 은 발행 승인 신호가 아니다.'
  echo '!!!! 발행 경로 사용 금지: release.yml·release-postprocess.py 는 이 플래그를 절대 싣지 않는다(테스트 핀).'
  cat <<'BANNER'
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
!!  ★게이트 강등 — spctl 평가 불능 (assessments disabled)
!!  이 실행은 Gatekeeper **실평가(spctl --assess)를 수행하지 못했다**.
!!  codesign 봉인 검증 + stapler 공증 티켓 검증 **단독 모드**로 내려간다.
!!  skip 이 아니다: ①②③⑤⑥ 는 그대로 필수이며 실패 시 exit 1 이다.
!!  그러나 "Gatekeeper 통과"는 이 실행으로 증명되지 않았다 — 판정 범위가 좁아졌다.
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
BANNER
  echo "::warning title=Gatekeeper 게이트 강등(진단 전용)::spctl assessments disabled — codesign/stapler 단독 모드로 평가함(실평가 미수행 · 발행 판정 아님)"
fi
echo

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
    # 봉인에 속하지 않는 파일이 실렸다면(added/modified/missing) 그 이름을 기계 라인으로 지목한다.
    print_seal_culprits "$CS_OUT" "$APP"
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
    echo "SKIP ④ spctl --assess($APP_NAME) — assessments disabled (진단 전용 강등 · --diagnose-degraded-ok · 위 배너 참조)"
  fi

  # ── ⑤ SEAL-2 불변식 전칭 정적 검사 — 원본 트리(마운트된 DMG 안 / 직접 지정 .app) 판독 전용 ──
  seal2_static_check "$APP_SRC"; SEAL2_RC=$?
  case "$SEAL2_RC" in
    0) SEAL2_TARGETS=$((SEAL2_TARGETS+1)) ;;
    3) info "⑤ SEAL-2($APP_NAME): python 런타임 미동봉 — 대상 아님(설치 도우미 등)" ;;
    *) echo; echo "=== 판정 불가 ==="; exit 2 ;;
  esac

  # ── ⑥ 첫-부팅 기록자 모사(설치 후 상태) — 설치 모사 사본($APP = 쓰기 가능 지반)을 대상으로 ──
  firstboot_sim_check "$APP"; SIM_RC=$?
  case "$SIM_RC" in
    0) SIM_TARGETS=$((SIM_TARGETS+1)) ;;
    3) info "⑥ 첫-부팅 모사($APP_NAME): Contents/MacOS/cys 부재 — 대상 아님(설치 도우미 등)" ;;
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
# ⑥ 을 한 앱에서도 못 돌렸는데 FAIL 도 0 이라면 "첫-부팅 기록자 모사 없는 PASS" 가 된다 — 측정
# 불능은 통과가 아니므로 폐쇄한다. FAIL 이 이미 있으면 판정(exit 1)은 성립한 것이라 그대로 둔다
# (합성 픽스처 하네스(test_release_postprocess_gate.py)의 '판정 도달' 계약도 이 조건이 보존한다 —
#  MacOS/cys 없는 픽스처 앱은 ②③ FAIL 로 exit 1 에 도달하고, 진짜 산출물의 진짜 PASS 만 ⑥ 을 요구).
if [ "$SIM_TARGETS" -eq 0 ] && [ "$FAIL_N" -eq 0 ]; then
  echo "✗ ⑥ 첫-부팅 모사: 평가한 앱 어디에도 Contents/MacOS/cys 가 없다 — 기록자 모사 0회로는 PASS 를 선언할 수 없다(레이아웃이 정말 바뀌었다면 게이트를 의도적으로 갱신하라)" >&2
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
  echo "✓ [진단 전용] 강등 모드 전 항목 PASS — Gatekeeper 실평가는 수행되지 않았다(발행 판정 아님 · --diagnose-degraded-ok · 배너 고지됨)"
else
  echo "✓ 전 항목 PASS — 격리된 사본이 Gatekeeper 실평가를 통과했다"
fi
# 기계 요약(마지막 줄 고정 · 헤더 종료 코드 항 참조) — CI 가 GITHUB_STEP_SUMMARY 로 승격한다.
echo "GATE_MODE=$MODE"
exit "$RC"
