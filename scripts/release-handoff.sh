#!/usr/bin/env bash
# Confidential workflow handoff for a public repository. GitHub Actions
# artifacts contain ciphertext only; plaintext exists only in runner-local
# directories and the collaborator-visible GitHub draft.
set -euo pipefail

usage() {
  echo "usage: $0 check | pack INPUT_DIR OUTPUT.tgz.enc | unpack INPUT.tgz.enc OUTPUT_DIR" >&2
  exit 2
}

require_key() {
  if [ -z "${RELEASE_HANDOFF_KEY:-}" ]; then
    echo "RELEASE_HANDOFF_KEY is required" >&2
    exit 1
  fi
  if [ "${#RELEASE_HANDOFF_KEY}" -lt 32 ]; then
    echo "RELEASE_HANDOFF_KEY must contain at least 32 characters" >&2
    exit 1
  fi
  command -v openssl >/dev/null || { echo "openssl is required" >&2; exit 2; }
  command -v tar >/dev/null || { echo "tar is required" >&2; exit 2; }
}

MODE="${1:-}"
require_key

case "$MODE" in
  check)
    [ "$#" -eq 1 ] || usage
    echo "release handoff key contract satisfied (value redacted)"
    ;;
  pack)
    [ "$#" -eq 3 ] || usage
    INPUT_DIR="$2"
    OUTPUT="$3"
    [ -d "$INPUT_DIR" ] || { echo "handoff input directory does not exist: $INPUT_DIR" >&2; exit 1; }
    [ ! -e "$OUTPUT" ] || { echo "handoff output already exists: $OUTPUT" >&2; exit 1; }
    mkdir -p "$(dirname "$OUTPUT")"
    PARTIAL="${OUTPUT}.partial.$$"
    trap 'rm -f "$PARTIAL"' EXIT
    COPYFILE_DISABLE=1 tar -C "$INPUT_DIR" -czf - . | openssl enc -aes-256-cbc -salt \
      -pbkdf2 -iter 200000 -md sha256 -pass env:RELEASE_HANDOFF_KEY \
      -out "$PARTIAL"
    [ -s "$PARTIAL" ] || { echo "encrypted handoff is empty" >&2; exit 1; }
    mv "$PARTIAL" "$OUTPUT"
    trap - EXIT
    echo "encrypted release handoff created: $(basename "$OUTPUT")"
    ;;
  unpack)
    [ "$#" -eq 3 ] || usage
    INPUT="$2"
    OUTPUT_DIR="$3"
    [ -s "$INPUT" ] || { echo "encrypted handoff does not exist or is empty: $INPUT" >&2; exit 1; }
    [ ! -e "$OUTPUT_DIR" ] || { echo "handoff output directory already exists: $OUTPUT_DIR" >&2; exit 1; }
    mkdir -p "$(dirname "$OUTPUT_DIR")"
    PARTIAL="${OUTPUT_DIR}.partial.$$"
    trap 'rm -rf "$PARTIAL"' EXIT
    mkdir "$PARTIAL"
    if ! openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -md sha256 \
      -pass env:RELEASE_HANDOFF_KEY -in "$INPUT" | tar -xzf - -C "$PARTIAL"; then
      echo "release handoff decryption failed" >&2
      exit 1
    fi
    mv "$PARTIAL" "$OUTPUT_DIR"
    trap - EXIT
    echo "encrypted release handoff unpacked: $(basename "$INPUT")"
    ;;
  *) usage ;;
esac
