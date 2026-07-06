#!/bin/sh
# IME 3차 수리 빌드 설치 — ★master 승인 게이트 뒤에만 실행 (앱 종료 상태 필수).
# 절차: 백업 → cys-dept 동봉(2차 탈락 전례 복원) → ditto 교체 → adhoc 서명 → quarantine 제거
#      → 계측 플래그 시딩(cysImeDebug=1) → 재기동은 주인님/master가 수동.
set -e
SRC="$HOME/.cys/src/cys-terminal"
BUNDLE="$SRC/target/release/bundle/macos/cys.app"
[ -d "$BUNDLE" ] || { echo "빌드 산출물 없음: $BUNDLE"; exit 1; }
if pgrep -qf "cys.app/Contents/MacOS/cys-app"; then
  echo "⚠ cys.app 실행 중 — GUI 종료 후 실행하라 (cysd 데몬은 유지 — 세션 생존)"; exit 1
fi

# 1) 현 설치본 백업 (타임스탬프)
BK="/Applications/cys.app.backup-pre-ime3-$(date +%H%M)"
ditto /Applications/cys.app "$BK"
echo "백업: $BK"

# 2) cys-dept 동봉 — 2차 설치 때 소리 없이 탈락한 전례 복원 (CLAUDE.md 전체경로 계약 준수)
cp "$SRC/cysjavis-pack/bin/cys-dept" "$BUNDLE/Contents/MacOS/cys-dept"
chmod +x "$BUNDLE/Contents/MacOS/cys-dept"

# 3) 교체 + 서명 + quarantine 제거
ditto "$BUNDLE" /Applications/cys.app
codesign --force --deep --sign - /Applications/cys.app
xattr -dr com.apple.quarantine /Applications/cys.app 2>/dev/null || true

# 4) 계측 플래그 시딩 (다음 기동부터 $TMPDIR/cys-ime.log 기록 — 근본원인 확정 캡처용)
sh "$SRC/scripts/seed-ime-debug.sh"

# 5) 검증
ls -la /Applications/cys.app/Contents/MacOS/
codesign -v /Applications/cys.app && echo "서명 OK"
echo "설치 완료 — cys.app 재기동 후 주인님 실타이핑으로 캡처·판정. 캡처 후 계측 끄기: seed-ime-debug.sh off (+재시작)"
