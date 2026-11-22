#!/bin/bash
# install-mac-devtools.sh — 동봉 런타임에 「개발자 도구 대역」을 깐다 (S1 버그2 · 오너 2026-11-22).
#
# 무엇을: <runtime>/devtools/usr/bin/xcrun (애플 셔임이 DEVELOPER_DIR 로 넘겨받는 대역 xcrun)
#         <runtime>/devtools/bin/git       (동봉 git 을 GIT_EXEC_PATH 와 함께 여는 런처)
#         <runtime>/python/pip.conf        (동봉 python 의 pip 가 앱 번들이 아니라 사용자 폴더에 설치하게 — 봉인 보호)
# 왜:     개발자 도구(Xcode·CLT)가 없는 맥의 /usr/bin/python3·/usr/bin/git 은 설치 창을 띄우는 껍데기다.
#         cys 는 그런 맥에서만 좌석 env 에 DEVELOPER_DIR=<runtime>/devtools 를 싣고(src/lib.rs spawn_env_pairs ⑧),
#         껍데기는 그 값을 보고 이 xcrun 을 대신 실행한다 → 좌석 안의 python3·git 이 앱에 든 것으로 풀린다.
#         규칙의 정의처는 src/macos_devtools.rs 머리말.
# 언제:   prep-mac-runtime.sh 끝에서, 그리고 build-macos-signed.sh 가 **조건 없이** 한 번 더 부른다 —
#         그 스크립트는 `.prep-target` 이 맞으면 prep 을 건너뛰므로, 이 변경 이전에 준비된 런타임 트리가
#         남아 있으면 대역 없는 앱이 조용히 나간다(선컴파일 SEAL-2 와 같은 이유).
# 원본은 scripts/mac-devtools/ 에 추적된다(내려받는 것이 아니다 — 네트워크 0).
#
# 사용: scripts/install-mac-devtools.sh <runtime 디렉터리>
set -euo pipefail
cd "$(dirname "$0")/.."
RT="${1:?usage: install-mac-devtools.sh <runtime dir>}"
SRC="scripts/mac-devtools"
[ -d "$RT" ] || { echo "✗ 런타임 디렉터리가 없다: $RT" >&2; exit 1; }
[ -d "$RT/python" ] || { echo "✗ 동봉 python 이 없다(런타임 준비가 먼저다): $RT/python" >&2; exit 1; }
for f in xcrun git; do
  [ -f "$SRC/$f" ] || { echo "✗ 대역 원본이 없다: $SRC/$f" >&2; exit 1; }
  /bin/sh -n "$SRC/$f" || { echo "✗ 대역 원본이 sh 문법 오류다: $SRC/$f" >&2; exit 1; }
done
[ -f "$SRC/pip.conf" ] || { echo "✗ pip 설정 원본이 없다: $SRC/pip.conf" >&2; exit 1; }
# 캐시된 런타임 트리에 남은 옛 판의 대역 파일이 섞이지 않게 대역 폴더는 매번 새로 짓는다(이 스크립트만 그 폴더에 쓴다).
DEV="${RT:?}/devtools"
echo "대역 폴더 재구성: $DEV"
rm -rf "${DEV:?}"
mkdir -p "$DEV/usr/bin" "$DEV/bin"
install -m 755 "$SRC/xcrun" "$DEV/usr/bin/xcrun"
install -m 755 "$SRC/git" "$DEV/bin/git"
install -m 644 "$SRC/pip.conf" "$RT/python/pip.conf"
# 깔린 것이 원본과 같은 바이트이고 실행 가능한지 확인한다(조용한 누락 금지).
cmp "$SRC/xcrun" "$DEV/usr/bin/xcrun"
cmp "$SRC/git" "$DEV/bin/git"
cmp "$SRC/pip.conf" "$RT/python/pip.conf"
[ -x "$DEV/usr/bin/xcrun" ] && [ -x "$DEV/bin/git" ] || { echo "✗ 대역에 실행 비트가 없다" >&2; exit 1; }
echo "✓ 개발자 도구 대역 설치: $DEV (xcrun · git 런처) + $RT/python/pip.conf"
