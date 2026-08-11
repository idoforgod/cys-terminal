#!/bin/bash
# installer-atomic-demo.sh — 복사경합 제거 + 봉인 승계 수동 실증 하니스(자기완결·ad-hoc 서명).
#
# 스파이크 프로토타입 demo.sh 를 레포 회귀로 편입. /Applications 를 읽지 않는다 — 임시 디렉터리에
# ad-hoc 서명 픽스처를 직접 만들어 돌리므로 어떤 맥에서도(설치 여부 무관) 재현된다.
#   DEMO 1: 나이브 직접 복사 = 복사 도중 최종 경로에 반쪽 번들 노출(드래그 경합 모델).
#   DEMO 2: 원자 설치(숨김 스테이징 → rename) = 최종 경로에 부분 노출 0.
#   DEMO 3: 봉인 승계 = ad-hoc 서명이 ditto → 원자 교체를 거쳐 codesign --deep --strict 통과로 따라온다.
# ★공증 티켓(spctl/stapler)은 실 Developer ID 서명·공증이 필요 → CI 서명 잡 몫. 여기선 봉인 승계만 본다.
set -uo pipefail

BASE="$(mktemp -d -t cys-installer-demo)"
SRC="$BASE/src/cys.app"
ts(){ /usr/bin/python3 -c 'import time;print("%.3f"%time.time())'; }
line(){ printf '%s\n' "----------------------------------------------------------------"; }

# ── ad-hoc 서명 소스 픽스처 만들기(실 Mach-O 실행물 + 필수 리소스 + Info.plist) ──
mkdir -p "$SRC/Contents/MacOS" "$SRC/Contents/Resources/runtime/python/bin"
cat > "$SRC/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.cysjavis.terminal.demo</string>
  <key>CFBundleExecutable</key><string>cys-app</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
for e in cys-app cysd cys; do cp /bin/echo "$SRC/Contents/MacOS/$e"; done
cp /bin/echo "$SRC/Contents/Resources/runtime/python/bin/python3"
: > "$SRC/Contents/Resources/pack.tar.gz"
: > "$SRC/Contents/Resources/pack-manifest.json"
codesign --force --deep --sign - "$SRC" >/dev/null 2>&1 || { echo "codesign 픽스처 실패"; exit 1; }
echo "픽스처(ad-hoc 서명): $SRC"

echo "########## DEMO 1: 나이브 직접 복사 = 부분 번들 노출(드래그 경합 모델) ##########"
D1="$BASE/direct/cys.app"; mkdir -p "$BASE/direct"
T0=$(ts)
ditto "$SRC" "$D1" &   # 최종 경로로 직접 복사(=Finder 드래그와 동일 위험)
P=$!
while kill -0 $P 2>/dev/null; do
  if codesign --verify --strict "$D1" >/dev/null 2>&1; then V=PASS; else V=FAIL; fi
  N=$(find "$D1" 2>/dev/null | wc -l | tr -d ' ')
  printf '  t+%5.1fs  최종경로 존재=%s  codesign(=Gatekeeper판정)=%s  files=%s\n' \
     "$(/usr/bin/python3 -c "print($(ts)-$T0)")" "$([ -e "$D1" ] && echo Y || echo N)" "$V" "$N"
  sleep 0.4
done
wait $P
if codesign --verify --strict "$D1" >/dev/null 2>&1; then VA=PASS; else VA=FAIL; fi
printf '  >> 복사 완료 후: codesign=%s  (복사 중엔 FAIL=사용자가 그 순간 실행하면 "손상")\n' "$VA"

line
echo "########## DEMO 2: 원자 설치(스테이징→rename) = 최종 경로에 부분 노출 0 ##########"
FINAL="$BASE/atomic/cys.app"; STG="$BASE/atomic/.cys.app.deploy-staging-$$"; mkdir -p "$BASE/atomic"
T0=$(ts)
ditto "$SRC" "$STG" &   # 숨김 스테이징으로 복사 — 최종 경로는 손대지 않음
P=$!
while kill -0 $P 2>/dev/null; do
  if [ -e "$FINAL" ]; then FE=Y; else FE=N; fi
  if codesign --verify --strict "$FINAL" >/dev/null 2>&1; then V=PASS; else V="N/A(최종경로 부재→실행할 창 없음)"; fi
  printf '  t+%5.1fs  최종경로(cys.app) 존재=%s  codesign(최종)=%s\n' \
     "$(/usr/bin/python3 -c "print($(ts)-$T0)")" "$FE" "$V"
  sleep 0.4
done
wait $P
TSWAP0=$(ts); mv "$STG" "$FINAL"; TSWAP1=$(ts)   # 신규 설치 = 단일 rename syscall = 원자
if codesign --verify --strict "$FINAL" >/dev/null 2>&1; then VA=PASS; else VA=FAIL; fi
printf '  >> rename 소요=%.4fs  직후 최종경로 codesign=%s (부분→완본 전이가 원자적)\n' \
   "$(/usr/bin/python3 -c "print($TSWAP1-$TSWAP0)")" "$VA"

line
echo "########## DEMO 3: 봉인 승계 — ad-hoc 서명이 ditto 복사·원자 교체로 따라오는가 ##########"
echo "  대상: $FINAL (DEMO2에서 원자 설치한 사본)"
codesign --verify --deep --strict --verbose=2 "$FINAL" 2>&1 | tail -2
codesign --verify --deep --strict "$FINAL" >/dev/null 2>&1 && echo "  ✓ codesign --deep --strict 통과(봉인 승계)" || echo "  ✗ codesign 실패"
echo "  --- spctl -a -t exec (실 공증이 아닌 ad-hoc → reject 예상 · 공증 승계는 CI 몫) ---"
spctl -a -t exec -vv "$FINAL" 2>&1 || true
echo "  --- 원본 무접촉 확인 ---"
codesign --verify --strict "$SRC" >/dev/null 2>&1 && echo "  ✓ 원본 $SRC 여전히 정상(읽기 소스로만 사용)"

line
echo "정리: rm -rf $BASE"
rm -rf "$BASE"
