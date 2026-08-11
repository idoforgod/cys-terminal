#!/bin/bash
# install-core.sh — 설치 도우미(Install cys.app)의 원자 설치 코어.
#
# ★로직 이중구현 금지: 원자 교체(ATOMIC-1)는 레포 정본 scripts/atomic_bundle.py(정본은 Rust
#   src/app_bundle.rs)를 **그대로** 부른다. 이 셸은 배선일 뿐이며 계약을 재구현하지 않는다.
#
# ★보안(LPE 방어): 이 스크립트도, 그것이 부르는 atomic_bundle.py 도 **자기 번들 내부**
#   (Contents/Resources)에서만 실행된다. 두 파일은 서명·공증된 설치 도우미 번들 안에 봉인돼 있어
#   설치 시점에 변조될 수 없다. 쓰기 가능한 DMG 형제·임의 경로의 스크립트를 root 로 실행하는 경로는
#   존재하지 않는다(installer.applescript 가 오직 이 번들 내부 경로만 호출).
#
# ★런타임: 설치 시점엔 cys 가 아직 설치돼 있지 않으므로 시스템 /usr/bin/python3 를 쓴다
#   (atomic_bundle.py 는 표준 라이브러리 + libc.renamex_np·codesign·ditto 만 사용 → 시스템 python 으로 충분).
#
# 사용: install-core.sh <SRC cys.app> <DEST cys.app>
# exit 0=원자 설치 완료 / 1=설치 실패(기존 번들 무접촉) / 2=소스 아님 / 3=번들 내부 도구 결손
set -euo pipefail

SRC="${1:?SRC (source cys.app) 필요}"
DEST="${2:?DEST (destination cys.app path) 필요}"
SRC="${SRC%/}"; DEST="${DEST%/}"

# 자기와 같은 디렉터리(=서명된 번들 내부 Contents/Resources)에서 정본 래퍼를 찾는다.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
WRAPPER="$SCRIPT_DIR/atomic_bundle.py"

[ -f "$WRAPPER" ] || { echo "✗ 번들 내부 원자 설치 도구 없음: $WRAPPER" >&2; exit 3; }
[ -d "$SRC/Contents" ] || { echo "✗ 소스가 .app 번들이 아님: $SRC" >&2; exit 2; }

# 정본 ATOMIC-1 집행부에 그대로 위임(exec — 종료 코드 그대로 전파).
exec /usr/bin/python3 "$WRAPPER" install "$SRC" "$DEST"
