#!/usr/bin/env bash
# build-macos-signed.sh — Apple Developer ID 서명 + 공증(notarization) + staple 자동 빌드.
# (오너 2026-06-15) ad-hoc 빌드는 다른 맥으로 전송하면 quarantine + 미공증으로 macOS가
# "손상됨(damaged)"으로 차단한다. Developer ID 인증서 + notarytool 자격증명이 있으면 Tauri가
# 빌드 중 자동으로 codesign(hardened runtime) + notarytool 공증 + stapler staple 한다.
# 이 스크립트는 자격증명을 fail-closed로 검증하고, 빌드 후 Gatekeeper 통과를 실측 확인한다.
#
# 사전(1회 셋업):
#   1) Apple Developer Program 가입($99/년) → "Developer ID Application" 인증서 발급·Keychain 설치
#   2) appleid.apple.com 에서 app-specific password 발급 (또는 App Store Connect API key)
#   3) 아래 env 설정
# 사용:
#   export APPLE_SIGNING_IDENTITY="Developer ID Application: NAME (TEAMID)"
#   export APPLE_ID="you@example.com" APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx" APPLE_TEAM_ID="TEAMID"
#   #   (또는 API key: APPLE_API_KEY_PATH=AuthKey_XXXX.p8 · APPLE_API_KEY=KEYID · APPLE_API_ISSUER=ISSUER)
#   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cys-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
#   scripts/build-macos-signed.sh
# exit 0=서명·공증·검증 통과 / 1=공증 검증 실패 / 2=자격증명·환경 미비
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
VERSION=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9][0-9.]*)".*/\1/')

# ── 타깃 아키텍처(무인자=호스트 네이티브 — arm64 경로 완전 불변) ──
# 사용: scripts/build-macos-signed.sh [aarch64-apple-darwin|x86_64-apple-darwin]
# arm64 호스트에서 x86_64를 넘기면 크로스빌드 — prep-mac-runtime.sh 도 같은 타깃으로 런타임을 교체한다.
TARGET="${1:-}"
if [ -z "$TARGET" ]; then
  case "$(uname -m)" in
    arm64)  TARGET=aarch64-apple-darwin ;;
    x86_64) TARGET=x86_64-apple-darwin ;;
    *) echo "unknown host arch $(uname -m)"; exit 2 ;;
  esac
fi
case "$TARGET" in
  aarch64-apple-darwin) DMG_ARCH=aarch64; DIST_ARCH=arm64 ;;   # Tauri DMG 명명 규칙과 일치
  x86_64-apple-darwin)  DMG_ARCH=x64;     DIST_ARCH=x64   ;;
  *) echo "지원하지 않는 타깃: $TARGET (aarch64-apple-darwin|x86_64-apple-darwin)"; exit 2 ;;
esac
# 호스트와 타깃이 다르면(크로스빌드) tauri build 에 --target 을 전달하고 산출물 경로에 타깃 세그먼트가 낀다.
HOST_ARCH_TARGET=$([ "$(uname -m)" = "arm64" ] && echo aarch64-apple-darwin || echo x86_64-apple-darwin)
if [ "$TARGET" != "$HOST_ARCH_TARGET" ]; then
  TAURI_TARGET_ARGS=(--target "$TARGET"); BUNDLE_BASE="target/$TARGET/release/bundle"
  # 크로스 빌드: bundle-prep.sh(beforeBuildCommand)가 사이드카 cys/cysd를 이 타깃으로
  # 크로스 빌드하도록 CYS_TARGET 전파 — 없으면 host(arm64) 사이드카가 실려 x64 앱이 깨진다.
  export CYS_TARGET="$TARGET"
else
  TAURI_TARGET_ARGS=(); BUNDLE_BASE="target/release/bundle"
fi
echo "== 대상 아키텍처: $TARGET (DMG=$DMG_ARCH · bundle=$BUNDLE_BASE) =="

# ── 자격증명 fail-closed 검증 ──
: "${APPLE_SIGNING_IDENTITY:?필요: export APPLE_SIGNING_IDENTITY='Developer ID Application: NAME (TEAMID)'}"
if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
  echo "공증 자격: notarytool keychain 프로파일($APPLE_NOTARY_PROFILE)"
elif [ -n "${APPLE_API_KEY:-}" ]; then
  : "${APPLE_API_ISSUER:?APPLE_API_KEY 사용 시 APPLE_API_ISSUER 필요}"
  echo "공증 자격: App Store Connect API key"
else
  : "${APPLE_ID:?공증용 Apple ID 필요 (또는 APPLE_NOTARY_PROFILE / APPLE_API_KEY)}"
  : "${APPLE_PASSWORD:?app-specific password 필요 (APPLE_PASSWORD)}"
  : "${APPLE_TEAM_ID:?APPLE_TEAM_ID 필요}"
  echo "공증 자격: Apple ID($APPLE_ID) + app-specific password"
fi
command -v xcrun >/dev/null || { echo "✗ Xcode Command Line Tools 필요(xcrun) — xcode-select --install"; exit 2; }
if ! security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
  echo "✗ Keychain에 'Developer ID Application' 인증서 없음 — Apple Developer에서 발급·설치 필요"; exit 2
fi
# ★fail-closed 승격(2026-07-10 · v0.12.35 빌드 1차 실패 원인): 구 경고문("미설정이어도 설치 DMG는 정상")은
# 실동작과 표류 — tauri.conf createUpdaterArtifacts 때문에 tauri build가 빌드 말미(~20분 후)에 hard-fail한다.
# 20분 낭비 대신 여기서 3초 만에 명확히 실패시킨다(다른 자격증명 검증과 동형).
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  echo "✗ TAURI_SIGNING_PRIVATE_KEY 미설정 — tauri build가 updater 아티팩트 서명에서 실패한다(빌드 말미 hard-fail)." >&2
  echo "  설정: export TAURI_SIGNING_PRIVATE_KEY=\"\$(cat ~/.tauri/cys-updater.key)\" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=\"\"" >&2
  exit 2
fi

# Browser Runtime qualification is a signed release input, never a mutable
# manifest flag. Hash every executable/tree and verify the freshly generated
# attestation/policy signatures before Tauri can copy resources into the app.
: "${CYS_BROWSER_RUNTIME_SECRET_KEY:?Browser Runtime minisign secret-key path required}"
: "${CYS_BROWSER_RUNTIME_PUBLIC_KEY:?Browser Runtime minisign public-key path required}"
: "${CYS_BROWSER_RUNTIME_KEY_ID:?Browser Runtime trust-root key id required}"
: "${CYS_BROWSER_RUNTIME_POLICY_EPOCH:?Browser Runtime policy epoch required}"
: "${CYS_BROWSER_RUNTIME_EXPIRES_AT:?Browser Runtime metadata expiry epoch required}"
# Signing mutates every executable/tree digest. It must complete before the
# final runtime lock hashes, attestation and policy minisign files are emitted.
bash scripts/runtime-stage-sign-macos.sh "$TARGET"
python3 scripts/browser-runtime-metadata.py prepare \
  --resource-root src-tauri/resources/browser-runtime \
  --target "$TARGET" \
  --key-id "$CYS_BROWSER_RUNTIME_KEY_ID" \
  --secret-key "$CYS_BROWSER_RUNTIME_SECRET_KEY" \
  --public-key "$CYS_BROWSER_RUNTIME_PUBLIC_KEY" \
  --trusted-keys cysjavis-pack/trusted-keys.json \
  --tauri-config src-tauri/tauri.conf.json \
  --policy-epoch "$CYS_BROWSER_RUNTIME_POLICY_EPOCH" \
  --expires-at "$CYS_BROWSER_RUNTIME_EXPIRES_AT"
export CYS_BROWSER_V2_RELEASE_QUALIFIED=1

# ── 동봉 런타임 준비 + inside-out 재서명 (RC-22/T6b — 공증 필수) ──
# tauri.conf.json bundle.resources("runtime/")는 Contents/Resources/runtime 으로 실리지만 Tauri는
# resources 내 Mach-O를 자동 서명하지 않는다(#12001) → ad-hoc(python·uv)/타팀(node) 서명 그대로면
# 공증이 "signature invalid / hardened runtime 미적용"으로 거부. tauri build **전에** 개별 재서명해
# .app 안으로 Developer ID 서명본이 실리게 한다. ★codesign --deep 금지(entitlement 오염·실행 차단) —
# inside-out(라이브러리 먼저·실행 바이너리 나중) 개별 서명. 인터프리터(python/node JIT)는 entitlements 적용.
# 런타임이 대상 타깃과 다른 아키텍처면(직전 빌드가 다른 arch) 강제 재준비 — arch 혼입 방지.
# .prep-target 마커로 현재 런타임 아키텍처를 추적한다(없으면 안전측 재준비).
RT_MARKER="src-tauri/runtime/.prep-target"
RT_CUR="$(cat "$RT_MARKER" 2>/dev/null || echo '')"
if [ ! -x "src-tauri/runtime/python/bin/python3" ] || [ "$RT_CUR" != "$TARGET" ]; then
  echo "== 동봉 런타임 준비($TARGET) =="
  bash scripts/prep-mac-runtime.sh "$TARGET"
  printf '%s' "$TARGET" > "$RT_MARKER"
fi
echo "== 동봉 runtime Mach-O inside-out 재서명 (Developer ID + hardened + timestamp) =="
ENT="src-tauri/entitlements.plist"
SIGN_N=0
# 1) 동적 라이브러리·로드가능 번들(.dylib/.so/.node) 먼저 — entitlements 불요
while IFS= read -r -d '' lib; do
  codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" "$lib"
  SIGN_N=$((SIGN_N+1))
done < <(find src-tauri/runtime \( -name '*.dylib' -o -name '*.so' -o -name '*.node' \) -type f -print0)
# 2) Mach-O 실행 바이너리(라이브러리 제외) — ★최소권한(codex T6b.1): 인터프리터(python·node V8 JIT)만
#    entitlements 적용, git/uv는 entitlements 없이 runtime 서명(경로 기반 분류·불필요 권한 확산 방지).
while IFS= read -r -d '' exe; do
  if file "$exe" | grep -q 'Mach-O'; then
    case "$exe" in
      src-tauri/runtime/python/*|src-tauri/runtime/node/*)
        codesign --force --timestamp --options runtime --entitlements "$ENT" --sign "$APPLE_SIGNING_IDENTITY" "$exe" ;;
      *)  # git·uv 등 — JIT/라이브러리검증 완화 불요 → entitlements 없이 hardened 서명
        codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" "$exe" ;;
    esac
    SIGN_N=$((SIGN_N+1))
  fi
done < <(find src-tauri/runtime -type f -perm +111 ! -name '*.dylib' ! -name '*.so' ! -name '*.node' -print0)
echo "  ✓ runtime Mach-O ${SIGN_N}개 재서명 (python/node=entitlements·git/uv=무 entitlements)"

# ── 앱 번들 빌드 (서명만 · 공증은 dedup 뒤로 1회 미룸) — RC-23 git-core dedup ──
# ★Tauri 번들러는 bundle.resources 디렉토리의 심볼릭링크를 역참조(dereference)한다(upstream #13219, 미해결).
#   dugite tar.gz는 libexec/git-core 빌트인 143개를 이미 `git` 심볼릭링크로 dedup(트리 141MB)하나, Tauri가
#   .app으로 복사하며 각 링크를 3.4MB 실복사본으로 부풀려 runtime/git 608MB(중복 464MB)·DMG 434MB가 된다.
#   → prep-mac-runtime.sh 단계 dedup은 무효(복사 시 재역참조). '.app 생성 후·서명 전' dedup만 유효하다(실측).
#   Tauri에 서명 신원만 주고 공증 자격은 감춰 '서명만' 시킨다(dedup이 서명 봉인을 깨므로 공증은 1회로 미룸).
#   fat DMG도 건너뛴다(--bundles app) — DMG는 dedup된 .app에서 hdiutil로 만든다
#   (`tauri build --bundles dmg`는 .app을 재빌드해 역참조를 되돌리므로 사용 불가 — 실측 확인).
echo "== 앱 번들 빌드(서명만·공증 보류) v$VERSION =="
env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID -u APPLE_API_KEY -u APPLE_API_ISSUER \
  bun x @tauri-apps/cli build ${TAURI_TARGET_ARGS[@]+"${TAURI_TARGET_ARGS[@]}"} --bundles app

APP="$BUNDLE_BASE/macos/cys.app"
DMG="$BUNDLE_BASE/dmg/cys_${VERSION}_${DMG_ARCH}.dmg"

# ── git-core 빌트인 dedup (Tauri 역참조 되돌리기) — 공유 스크립트로 통일 ──
# 로직·기준점(libexec/git-core/git)·자기제외·동일디렉토리 링크·잔존 중복본 가드는
# scripts/dedup-git-core.sh 단일 출처(.github/workflows/release.yml CI 경로와 공유 — 드리프트 방지).
echo "== runtime/git dedup (git-core 빌트인 → 동일 디렉토리 git 심볼릭링크) =="
bash scripts/dedup-git-core.sh "$APP"

# dedup은 Resources를 바꿔 Tauri가 봉인한 외부 앱 서명을 깬다 → 외부 앱 서명만 재봉인(--force · ★--deep 금지).
# 중첩 Mach-O(pre-sign된 runtime bin/git·Tauri가 서명한 sidecar/framework/메인바이너리)는 그대로 유효하다.
echo "== dedup 후 외부 앱 서명 재봉인 (inside-out 유지·--deep 금지) =="
codesign --force --options runtime --timestamp --entitlements "$ENT" --sign "$APPLE_SIGNING_IDENTITY" "$APP"

# ★동봉 Mach-O 포함 앱 전체 서명 무결성 검증 (공증 제출 전) — dedup 심볼릭링크 포함 sealed resource 검증.
# --deep는 *검증 전용*으로만 사용(D-4 규칙 유지 — 서명은 inside-out 개별, 검증은 --deep 허용). 실측: 링크 트리 통과.
echo "== 동봉 서명 검증: codesign --verify --deep --strict (제출 전) =="
if codesign --verify --deep --strict --verbose=4 "$APP" 2>&1; then
  echo "  ✓ codesign --verify --deep --strict 통과 (dedup 링크 포함 중첩 서명 무결)"
else
  echo "  ✗ 서명 검증 실패 — 중첩 바이너리 서명 결손. 위 로그의 offender 재서명 필요"; exit 1
fi

# ── 공증(1회) + staple + DMG/업데이터 재생성 ──
# ⚠ 아래 notarytool/stapler/signer 경로는 Apple Developer 자격·업데이터 키가 필요해 워커 환경에선 실행 검증
#   불가 — dedup/재서명/hdiutil UDZO/codesign --verify 는 실측 통과. 오너 자격으로 최종 실행 검증을 요한다.
# notarytool 자격: APPLE_NOTARY_PROFILE(keychain 프로파일·오너 로컬 기본) > APPLE_API_KEY > APPLE_ID 3원 (상단서 fail-closed 검증됨).
if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
  NOTARY_ARGS=(--keychain-profile "$APPLE_NOTARY_PROFILE")
elif [ -n "${APPLE_API_KEY:-}" ]; then
  NOTARY_ARGS=(--key "${APPLE_API_KEY_PATH:?APPLE_API_KEY 사용 시 APPLE_API_KEY_PATH(.p8) 필요}" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
else
  NOTARY_ARGS=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
fi

echo "== 공증: 앱(zip 제출) → staple (dmg 안에 staple된 앱을 담아 offline 검증까지 견고) =="
APPZIP="$APP.notarize.zip"
ditto -c -k --keepParent "$APP" "$APPZIP"
xcrun notarytool submit "$APPZIP" "${NOTARY_ARGS[@]}" --wait
xcrun stapler staple "$APP"
rm -f "$APPZIP"

# 업데이터 아티팩트 재생성(dedup 반영): Tauri가 만든 fat cys.app.tar.gz(447MB)를 dedup·staple된 앱으로
# 다시 tar(심볼릭링크 보존 → 다운로드 축소)하고 업데이터 키로 재서명. make-update-manifest.sh가 이 .sig를 읽는다.
# (주의: Tauri 업데이터 *클라이언트*의 심볼릭링크 보존은 버전의존 #7480 — 다운로드는 축소되나 설치 후 on-disk
#  크기는 클라이언트 tauri 동작에 좌우될 수 있음. 신규 설치 경로인 DMG는 확실히 축소된다.)
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  echo "== 업데이터 tar.gz 재생성(dedup 반영) + 재서명 =="
  ( cd "$(dirname "$APP")" && tar czf cys.app.tar.gz cys.app )
  bun x @tauri-apps/cli signer sign --private-key "$TAURI_SIGNING_PRIVATE_KEY" --password "" "$APP.tar.gz"
fi

echo "== dedup·staple된 앱으로 UDZO DMG 생성(hdiutil — Tauri 기본 포맷과 동일) =="
DMGSTAGE="$(mktemp -d)"; ditto "$APP" "$DMGSTAGE/cys.app"; ln -s /Applications "$DMGSTAGE/Applications"
mkdir -p "$(dirname "$DMG")"
hdiutil create -volname "cys" -srcfolder "$DMGSTAGE" -ov -format UDZO "$DMG"
rm -rf "$DMGSTAGE"
# DMG 자체도 Developer ID 서명 — 구 Tauri 흐름과 패리티. 서명 없으면 spctl -t open(primary-signature)
# 이 'no usable signature'로 거부한다(2026-07-04 실측). 서명은 반드시 notarytool 제출 전(CDHash 결속).
codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"

echo "== 공증: DMG 제출 → staple =="
xcrun notarytool submit "$DMG" "${NOTARY_ARGS[@]}" --wait
xcrun stapler staple "$DMG"

echo "== 검증: Gatekeeper(spctl) + 공증 티켓(stapler) =="
if spctl -a -vv "$APP" 2>&1 | grep -qi "accepted"; then
  echo "  ✓ spctl: accepted (다른 맥에서도 경고 없이 열림 — '손상됨' 해소)"
else
  echo "  ✗ spctl 거부 — 공증 실패. 위 notarytool 결과를 확인하라"; exit 1
fi
if xcrun stapler validate "$APP" >/dev/null 2>&1; then
  echo "  ✓ app 공증 티켓 stapled"
else
  echo "  ✗ app staple 검증 실패" >&2
  exit 1
fi
if xcrun stapler validate "$DMG" >/dev/null 2>&1; then
  echo "  ✓ DMG 공증 티켓 stapled"
else
  echo "  ✗ DMG staple 검증 실패" >&2
  exit 1
fi
# DMG 자체 Gatekeeper 게이트 — .app spctl만으론 DMG 서명 누락을 못 잡는다(2026-07-04 실측 갭).
if spctl -a -t open --context context:primary-signature -vv "$DMG" 2>&1 | grep -qi "accepted"; then
  echo "  ✓ DMG spctl: accepted (primary-signature)"
else
  echo "  ✗ DMG spctl 거부 — DMG codesign/공증 확인 필요"; exit 1
fi

echo "== 배포본 정리 + 자동업데이트 매니페스트 =="
mkdir -p dist-mac
cp "$DMG" "dist-mac/cys-${VERSION}-macos-${DIST_ARCH}.dmg"
CYS_RELEASE_BUNDLE_ROOT="$BUNDLE_BASE" \
  sh scripts/make-update-manifest.sh \
    "$VERSION" idoforgod cys-terminal "$TARGET"
echo "✓ 공증 빌드 완료: dist-mac/cys-${VERSION}-macos-${DIST_ARCH}.dmg"
echo "  → ad-hoc 재서명·xattr 우회 불필요. gh release 발행은 오너 승인 후."
