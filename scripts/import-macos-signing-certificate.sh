#!/usr/bin/env bash
# Import a Developer ID certificate into an ephemeral CI keychain.
# Secret values are consumed only by security/base64 and are never printed.
set -euo pipefail

python3 "$(dirname "$0")/release-credentials-check.py" macos >/dev/null

KEYCHAIN_PATH="${RUNNER_TEMP:?RUNNER_TEMP is required}/cys-release-signing.keychain-db"
P12_PATH="$(mktemp "${RUNNER_TEMP}/cys-release-cert.XXXXXX.p12")"
trap 'rm -f "$P12_PATH"' EXIT

printf '%s' "$APPLE_CERTIFICATE_B64" | base64 --decode > "$P12_PATH"
security create-keychain -p "$APPLE_KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$APPLE_KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security import "$P12_PATH" -k "$KEYCHAIN_PATH" \
  -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
  -k "$APPLE_KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH" >/dev/null
security list-keychains -d user -s "$KEYCHAIN_PATH"

security find-identity -v -p codesigning "$KEYCHAIN_PATH" 2>/dev/null \
  | grep -Fq "$APPLE_SIGNING_IDENTITY" || {
    echo "configured Developer ID identity was not imported" >&2
    exit 1
  }
echo "macOS signing identity imported into ephemeral keychain"
