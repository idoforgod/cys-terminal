#!/usr/bin/env bash
# Inside-out Developer ID signing for the staged Browser Runtime target.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
TARGET="${1:?usage: runtime-stage-sign-macos.sh <target>}"
case "$TARGET" in
  aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "unsupported macOS Browser Runtime target: $TARGET" >&2; exit 2 ;;
esac
: "${APPLE_SIGNING_IDENTITY:?Developer ID signing identity is required}"

TARGET_ROOT="${2:-src-tauri/resources/browser-runtime/runtime/$TARGET}"
ENTITLEMENTS="src-tauri/entitlements.plist"
test -d "$TARGET_ROOT" || { echo "Browser Runtime target tree missing: $TARGET_ROOT" >&2; exit 2; }
test -f "$ENTITLEMENTS" || { echo "Browser Runtime entitlements missing" >&2; exit 2; }
if find "$TARGET_ROOT" -type l -print -quit | grep -q .; then
  echo "Browser Runtime target tree contains a symlink" >&2
  exit 1
fi

ORDER_FILE="$(mktemp "${TMPDIR:-/tmp}/cys-browser-sign-order.XXXXXX")"
trap 'rm -f "$ORDER_FILE"' EXIT

# Sign every Mach-O leaf deepest-first. V8/JIT executables (engine and Chromium
# helpers) receive the app entitlements; dylibs/framework payloads and the Rust
# supervisor remain least-privilege hardened signatures.
python3 - "$TARGET_ROOT" > "$ORDER_FILE" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
for path in sorted((p for p in root.rglob("*") if p.is_file()), key=lambda p: (-len(p.parts), p.as_posix())):
    print(path)
PY
MACH_COUNT=0
while IFS= read -r binary; do
  if ! file "$binary" | grep -q 'Mach-O'; then
    continue
  fi
  case "$binary" in
    */supervisor/cys-browserd|*.dylib|*.so|*.node)
      codesign --force --timestamp --options runtime \
        --sign "$APPLE_SIGNING_IDENTITY" "$binary"
      ;;
    *)
      codesign --force --timestamp --options runtime --entitlements "$ENTITLEMENTS" \
        --sign "$APPLE_SIGNING_IDENTITY" "$binary"
      ;;
  esac
  MACH_COUNT=$((MACH_COUNT + 1))
done < "$ORDER_FILE"
test "$MACH_COUNT" -gt 2 || { echo "Browser Runtime Mach-O set is incomplete" >&2; exit 1; }

# Seal nested code bundles after their leaves; the outer Chromium.app is last.
python3 - "$TARGET_ROOT" > "$ORDER_FILE" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
bundles = [p for p in root.rglob("*") if p.is_dir() and p.suffix in (".app", ".framework", ".xpc")]
for path in sorted(bundles, key=lambda p: (-len(p.parts), p.as_posix())):
    print(path)
PY
BUNDLE_COUNT=0
while IFS= read -r bundle; do
  case "$bundle" in
    *.framework)
      codesign --force --timestamp --options runtime \
        --sign "$APPLE_SIGNING_IDENTITY" "$bundle"
      ;;
    *)
      codesign --force --timestamp --options runtime --entitlements "$ENTITLEMENTS" \
        --sign "$APPLE_SIGNING_IDENTITY" "$bundle"
      ;;
  esac
  BUNDLE_COUNT=$((BUNDLE_COUNT + 1))
done < "$ORDER_FILE"
test -f "$TARGET_ROOT/chromium/chrome-mac/headless_shell" || {
  echo "staged Chromium headless shell is missing" >&2; exit 1;
}

# Verify the exact staged set before hashes and minisign metadata are produced.
while IFS= read -r binary; do
  if file "$binary" | grep -q 'Mach-O'; then
    codesign --verify --strict --verbose=2 "$binary"
  fi
done < <(find "$TARGET_ROOT" -type f | sort)
while IFS= read -r app; do
  codesign --verify --deep --strict --verbose=2 "$app"
done < <(find "$TARGET_ROOT" -type d -name '*.app' | sort)

echo "Browser Runtime Developer ID signing verified: target=$TARGET mach_o=$MACH_COUNT bundles=$BUNDLE_COUNT"
