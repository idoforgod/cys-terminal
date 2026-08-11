#!/usr/bin/env bash
# Import a Developer ID certificate into an ephemeral CI keychain.
# Secret values are consumed only by security/base64 and are never printed.
#
# v0.14.0 백포트 주(2026-07-28): v0.13.23 원본에서 release-credentials-check.py 호출만 제거.
# (그 스크립트는 이 브랜치에 없다 — 자격증명 fail-closed는 build-macos-signed.sh와
#  아래 ${VAR:?} 확장이 담당한다. 오너 확정 §16-③)
set -euo pipefail

: "${APPLE_CERTIFICATE_B64:?APPLE_CERTIFICATE_B64 required}"
: "${APPLE_CERTIFICATE_PASSWORD:?APPLE_CERTIFICATE_PASSWORD required}"
: "${APPLE_KEYCHAIN_PASSWORD:?APPLE_KEYCHAIN_PASSWORD required}"
: "${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY required}"

KEYCHAIN_PATH="${RUNNER_TEMP:?RUNNER_TEMP is required}/cys-release-signing.keychain-db"
P12_PATH="$(mktemp "${RUNNER_TEMP}/cys-release-cert.XXXXXX.p12")"
INSTALLER_P12_PATH="$(mktemp "${RUNNER_TEMP}/cys-release-installer-cert.XXXXXX.p12")"
trap 'rm -f "$P12_PATH" "$INSTALLER_P12_PATH"' EXIT

printf '%s' "$APPLE_CERTIFICATE_B64" | base64 --decode > "$P12_PATH"
security create-keychain -p "$APPLE_KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$APPLE_KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security import "$P12_PATH" -k "$KEYCHAIN_PATH" \
  -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security

# ── (조건부) Developer ID Installer 인증서 import — productsign/.pkg 서명용 ──
# APPLE_INSTALLER_CERTIFICATE_B64(+_PASSWORD)가 있을 때만 같은 ephemeral 키체인에 추가한다.
# 부재 시 조용히 skip → 현행 Application(코드서명) 경로는 완전 무변경(기존 DMG 빌드 무손상).
# set-key-partition-list 전에 import 해야 아래 한 번의 partition-list 호출이 두 키를 함께 덮는다.
if [ -n "${APPLE_INSTALLER_CERTIFICATE_B64:-}" ]; then
  : "${APPLE_INSTALLER_CERTIFICATE_PASSWORD:?APPLE_INSTALLER_CERTIFICATE_B64 사용 시 APPLE_INSTALLER_CERTIFICATE_PASSWORD 필요}"
  printf '%s' "$APPLE_INSTALLER_CERTIFICATE_B64" | base64 --decode > "$INSTALLER_P12_PATH"
  security import "$INSTALLER_P12_PATH" -k "$KEYCHAIN_PATH" \
    -P "$APPLE_INSTALLER_CERTIFICATE_PASSWORD" -T /usr/bin/productsign -T /usr/bin/security
  echo "Developer ID Installer 인증서 import (productsign 허용)"
fi

security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
  -k "$APPLE_KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH" >/dev/null
security list-keychains -d user -s "$KEYCHAIN_PATH"

security find-identity -v -p codesigning "$KEYCHAIN_PATH" 2>/dev/null \
  | grep -Fq "$APPLE_SIGNING_IDENTITY" || {
    echo "configured Developer ID identity was not imported" >&2
    exit 1
  }
echo "macOS signing identity imported into ephemeral keychain"

# ── (조건부) Installer 신원 검증 — .pkg 서명에 쓸 identity가 실제로 들어왔는지 fail-closed ──
# 설치관리자(Developer ID Installer) 인증서는 codesigning 정책이 아니라 전체 identity 목록에 뜬다.
if [ -n "${APPLE_INSTALLER_SIGNING_IDENTITY:-}" ]; then
  security find-identity -v "$KEYCHAIN_PATH" 2>/dev/null \
    | grep -Fq "$APPLE_INSTALLER_SIGNING_IDENTITY" || {
      echo "configured Developer ID Installer identity was not imported" >&2
      exit 1
    }
  echo "macOS installer signing identity imported into ephemeral keychain"
fi
