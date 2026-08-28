#!/usr/bin/env bash
# nsis-hook compile harness runner.
#
#   bash scripts/tests/nsis-hook-compile/run.sh   -> exit 0 = the real hook compiles
#
# What it does (works on macOS/Linux with POSIX makensis, and on Windows CI):
#   1. copies src-tauri/nsis-hooks.nsh byte-identically into build/ (the hook resolves its
#      sidecar sources via ${__FILEDIR__}\binaries\..., so the copy gets its own binaries/)
#   2. generates fake PE32+ executables WITH a real VS_VERSIONINFO resource
#      (make_versioned_pe.py) so the hook's `!getdllversion /packed` oracle runs for real
#      — a missing resource must fail the compile (no /noerrors), and this harness proves
#      that path stays exercised
#   3. compiles harness.nsi with warnings-as-errors (-WX), mirroring the template's
#      include order and macro insertion (an uninserted macro body is never compiled)
#
# Env overrides:
#   MAKENSIS            makensis binary (default: makensis on PATH)
#   CYS_HARNESS_VERSION version stamped into the fake PEs (default 0.14.28.0)
#   PYTHON              python interpreter (default: python3)
set -euo pipefail
cd "$(dirname "$0")"

MAKENSIS="${MAKENSIS:-makensis}"
PYTHON="${PYTHON:-python3}"
VER="${CYS_HARNESS_VERSION:-0.14.28.0}"
HOOK="../../../src-tauri/nsis-hooks.nsh"

command -v "$MAKENSIS" >/dev/null 2>&1 || { echo "FAIL: makensis not found (set MAKENSIS=...)" >&2; exit 2; }
[ -f "$HOOK" ] || { echo "FAIL: hook not found at $HOOK" >&2; exit 2; }

rm -rf build
mkdir -p build/binaries
cp "$HOOK" build/nsis-hooks.nsh
cmp -s "$HOOK" build/nsis-hooks.nsh || { echo "FAIL: hook copy not byte-identical" >&2; exit 2; }

"$PYTHON" make_versioned_pe.py --version "$VER" --out build/binaries/cys-x86_64-pc-windows-msvc.exe
"$PYTHON" make_versioned_pe.py --version "$VER" --out build/binaries/cysd-x86_64-pc-windows-msvc.exe
"$PYTHON" make_versioned_pe.py --version "$VER" --out build/binaries/cys-app.exe

# -WX: any makensis warning fails the harness (the real build treats hook warnings as
# failures too — see IMPL-SPEC W5-2). -INPUTCHARSET UTF8 mirrors the tauri bundler.
"$MAKENSIS" -V2 -WX -INPUTCHARSET UTF8 harness.nsi

[ -f build/harness-setup.exe ] || { echo "FAIL: harness-setup.exe not produced" >&2; exit 2; }
echo "nsis-hook-compile: OK (hook compiled, macros inserted, getdllversion oracle exercised)"
