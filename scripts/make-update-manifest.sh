#!/bin/sh
# Emit one architecture-correct updater fragment and normalized updater assets.
# The release assembler is the sole writer that merges these fragments into the
# final latest.json. No signing credential is read here.
set -eu

cd "$(dirname "$0")/.."

VERSION="${1:?usage: make-update-manifest.sh <version> <owner> [repo] [target]}"
OWNER="${2:?owner required}"
REPO="${3:-cys-terminal}"
TARGET="${4:-}"

if [ -z "$TARGET" ]; then
  case "$(uname -m)" in
    arm64) TARGET=aarch64-apple-darwin ;;
    x86_64) TARGET=x86_64-apple-darwin ;;
    *) echo "unsupported macOS architecture: $(uname -m)" >&2; exit 2 ;;
  esac
fi

case "$TARGET" in
  aarch64-apple-darwin)
    ARCH=aarch64
    PLATFORM=darwin-aarch64
    ;;
  x86_64-apple-darwin)
    ARCH=x64
    PLATFORM=darwin-x86_64
    ;;
  *)
    echo "unsupported updater target: $TARGET" >&2
    exit 2
    ;;
esac

if [ -n "${CYS_RELEASE_BUNDLE_ROOT:-}" ]; then
  BUNDLE="$CYS_RELEASE_BUNDLE_ROOT"
elif [ -d "target/$TARGET/release/bundle/macos" ]; then
  BUNDLE="target/$TARGET/release/bundle"
else
  BUNDLE="target/release/bundle"
fi
OUTPUT="${CYS_RELEASE_OUTPUT_DIR:-dist-update}"
TARBALL="$BUNDLE/macos/cys.app.tar.gz"
SIG_FILE="$TARBALL.sig"

[ -s "$TARBALL" ] || { echo "missing updater archive: $TARBALL" >&2; exit 1; }
[ -s "$SIG_FILE" ] || { echo "missing updater signature: $SIG_FILE" >&2; exit 1; }
mkdir -p "$OUTPUT"

ASSET="cys_${VERSION}_${ARCH}.app.tar.gz"
SIG_ASSET="$ASSET.sig"
cp "$TARBALL" "$OUTPUT/$ASSET"
cp "$SIG_FILE" "$OUTPUT/$SIG_ASSET"

PUBDATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
python3 - "$VERSION" "$OWNER" "$REPO" "$PLATFORM" "$ASSET" \
  "$OUTPUT/$SIG_ASSET" "$OUTPUT/latest-$PLATFORM.json" "$PUBDATE" <<'PY'
import json
import pathlib
import sys

version, owner, repo, platform, asset, sig_path, output, pub_date = sys.argv[1:]
signature = pathlib.Path(sig_path).read_text(encoding="utf-8").strip()
if not signature:
    raise SystemExit("empty updater signature")
payload = {
    "version": version,
    "notes": f"cys {version}",
    "pub_date": pub_date,
    "platforms": {
        platform: {
            "signature": signature,
            "url": f"https://github.com/{owner}/{repo}/releases/download/v{version}/{asset}",
        }
    },
}
pathlib.Path(output).write_text(
    json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

echo "updater fragment complete: $OUTPUT/latest-$PLATFORM.json"
