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

# ── 실행 인터프리터 선택 (진위 검증을 통과한 소스의 동봉 python 우선) ──
SRC_PY="$SRC/Contents/Resources/runtime/python/bin/python3"
if [ -x "$SRC_PY" ]; then
  PY="$SRC_PY"
else
  PY="/usr/bin/python3"   # 폴백(소스 동봉 python 결손 시) — CLT 미설치 맥에선 실패할 수 있음(finding 7)
fi

# 정본 ATOMIC-1 집행부에 그대로 위임(exec — 종료 코드 그대로 전파).
# PYTHONDONTWRITEBYTECODE=1: 읽기 전용 DMG 소스/설치 위치에 .pyc 를 쓰려는 시도 자체를 차단(봉인 보호).
export PYTHONDONTWRITEBYTECODE=1
exec "$PY" "$WRAPPER" install "$SRC" "$DEST"
