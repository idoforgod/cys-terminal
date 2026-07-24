#!/usr/bin/env bash
# Download and hash every byte in the reviewed public homepage release set.
# This command is read-only and never loads release credentials.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION=""
RELEASE_DIR=""
SITE_ROOT=""

usage() {
  echo "usage: $0 --version X.Y.Z --release-dir DIR --site-root https://host" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --release-dir) RELEASE_DIR="${2:-}"; shift 2 ;;
    --site-root) SITE_ROOT="${2:-}"; shift 2 ;;
    *) usage ;;
  esac
done

if [ -z "$VERSION" ] || [ -z "$RELEASE_DIR" ] || [ -z "$SITE_ROOT" ]; then usage; fi
case "$SITE_ROOT" in
  https://*) ;;
  http://127.0.0.1:*|http://localhost:*) ;;
  file://*)
    [ "${CYS_RELEASE_VERIFY_ALLOW_FILE:-}" = "1" ] || {
      echo "file transport is test-only" >&2
      exit 2
    }
    ;;
  *)
    echo "site root must use https (loopback http is test-only): $SITE_ROOT" >&2
    exit 2
    ;;
esac

python3 "$ROOT_DIR/scripts/release-verify.py" \
  --version "$VERSION" --release-dir "$RELEASE_DIR" >/dev/null

command -v curl >/dev/null || { echo "curl is required" >&2; exit 2; }
if command -v shasum >/dev/null; then
  hash_file() { shasum -a 256 "$1" | awk '{print $1}'; }
elif command -v sha256sum >/dev/null; then
  hash_file() { sha256sum "$1" | awk '{print $1}'; }
else
  echo "shasum or sha256sum is required" >&2
  exit 2
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
BASE="${SITE_ROOT%/}/downloads"

curl --fail --silent --show-error --location "$BASE/index.html" -o "$TMP_DIR/index.html"
grep -Fq 'data-cys-release-marker="windows-defender-guidance-v1"' "$TMP_DIR/index.html" || {
  echo "remote downloads index is missing the Defender marker" >&2
  exit 1
}
grep -Fq 'Windows Defender' "$TMP_DIR/index.html" || {
  echo "remote downloads index is missing Windows Defender guidance" >&2
  exit 1
}
if grep -Fq 'xattr -d' "$TMP_DIR/index.html"; then
  echo "remote downloads index contains a forbidden macOS quarantine bypass" >&2
  exit 1
fi

curl --fail --silent --show-error --location \
  "$BASE/recovery/index.html" -o "$TMP_DIR/recovery.html"
grep -Fq 'data-cys-release-marker="release-recovery-v1"' "$TMP_DIR/recovery.html" || {
  echo "remote recovery index is missing the recovery marker" >&2
  exit 1
}

curl --fail --silent --show-error --location \
  "$BASE/SHA256SUMS.txt" -o "$TMP_DIR/SHA256SUMS.txt"
cmp -s "$RELEASE_DIR/SHA256SUMS.txt" "$TMP_DIR/SHA256SUMS.txt" || {
  echo "remote SHA256SUMS.txt differs from the reviewed release" >&2
  exit 1
}

COUNT=0
while IFS= read -r ROW; do
  EXPECTED="${ROW%%  *}"
  FILENAME="${ROW#*  }"
  case "$FILENAME" in
    ""|*/*|*\\*|.*) echo "unsafe checksum filename: $FILENAME" >&2; exit 1 ;;
  esac
  curl --fail --silent --show-error --location \
    "$BASE/$FILENAME" -o "$TMP_DIR/$FILENAME"
  ACTUAL="$(hash_file "$TMP_DIR/$FILENAME")"
  [ "$ACTUAL" = "$EXPECTED" ] || {
    echo "remote checksum mismatch: $FILENAME" >&2
    exit 1
  }
  COUNT=$((COUNT + 1))
done < "$TMP_DIR/SHA256SUMS.txt"

echo "remote release verified: version=$VERSION assets=$COUNT"
