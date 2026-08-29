#!/usr/bin/env bash
# nsis-hook compile harness runner.
#
#   bash scripts/tests/nsis-hook-compile/run.sh   -> exit 0 = the real hook compiles AND
#                                                    every compile-time guard still fires
#
# What it does (works on macOS/Linux with POSIX makensis, and on Windows CI):
#   1. copies src-tauri/nsis-hooks.nsh byte-identically into build/ (the hook resolves its
#      sidecar sources via ${__FILEDIR__}\binaries\..., so the copy gets its own binaries/)
#   2. generates fake PE32+ executables WITH a real VS_VERSIONINFO resource
#      (make_versioned_pe.py) so the hook's `!getdllversion /packed` oracle runs for real
#   3. compiles harness.nsi with warnings-as-errors (-WX), mirroring the template's
#      include order and macro insertion (an uninserted macro body is never compiled)
#   4. ★negative controls (R2 round-2): re-compiles against poisoned inputs and DEMANDS
#      failure with the exact NSIS-CONTRACT §5 token. A green positive compile alone cannot
#      notice the guards being deleted from the hook (measured: an E1 mutation stripping all
#      four !error guards still exited 0) — only a red negative can.
#        N1 sidecar without VERSIONINFO      -> must fail: 'refusing to build a blind oracle'
#        N2 sidecar stamped 0.0.0.0          -> must fail: 'version resource regression'
#        N3 sidecar missing at compile time  -> must fail: 'sidecar source not found at compile time'
#        N4 the three §5 tokens exist verbatim in the REAL hook — protects the grep belts
#           in release.yml / windows-build.yml (their semantics: token in build log = fail;
#           if the hook's wording drifts, those belts go silently blind)
#        N5 CONTRACT §6 taskkill census on the REAL hook — 9 kills inside
#           NSIS_HOOK_PREUNINSTALL / 10 in the whole file; the single one outside must be
#           the GUI-only '/F /IM cys-app.exe' with no '/T' (owner anchor ④: no path may
#           kill every pane's agent — the deleted v0.14.27 kill fallback class)
#        N6 checklist anchor pins — every hook anchor cited by
#           docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md exists in the hook, and raw ':NNN'
#           hook line-number citations are refused (they rotted twice — R2/R3)
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

SIDE_CYS=build/binaries/cys-x86_64-pc-windows-msvc.exe
SIDE_CYSD=build/binaries/cysd-x86_64-pc-windows-msvc.exe
"$PYTHON" make_versioned_pe.py --version "$VER" --out "$SIDE_CYS"
"$PYTHON" make_versioned_pe.py --version "$VER" --out "$SIDE_CYSD"
"$PYTHON" make_versioned_pe.py --version "$VER" --out build/binaries/cys-app.exe

# ── positive: the real hook must compile ──────────────────────────────────────
# -WX: any makensis warning fails the harness (the real build treats hook warnings as
# failures too — see IMPL-SPEC W5-2). -INPUTCHARSET UTF8 mirrors the tauri bundler.
"$MAKENSIS" -V2 -WX -INPUTCHARSET UTF8 harness.nsi
[ -f build/harness-setup.exe ] || { echo "FAIL: harness-setup.exe not produced" >&2; exit 2; }

# ── negative controls: each poisoned input must fail with the CONTRACT §5 token ──
# The token strings are the frozen NSIS-CONTRACT.md §5 '컴파일 실패 문자열' — verbatim,
# no invention. A negative that exits 0, or fails WITHOUT its token, is a guard regression.
expect_fail_with() { # $1=token  $2=label
  local token="$1" label="$2" out rc
  set +e
  out=$("$MAKENSIS" -V2 -WX -INPUTCHARSET UTF8 harness.nsi 2>&1)
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    echo "FAIL[$label]: poisoned input COMPILED (exit 0) — the compile-time guard is gone" >&2
    exit 1
  fi
  if ! printf '%s' "$out" | grep -qF "$token"; then
    echo "FAIL[$label]: compile failed but WITHOUT the contract token '$token' — wording drift disables the CI grep belts" >&2
    printf '%s\n' "$out" | tail -20 >&2
    exit 1
  fi
  echo "nsis-hook-compile: $label OK (refused with '$token')"
}

# N1 — versionless PE sidecar: POSIX makensis defines EMPTY values instead of erroring
# (measured), so only the hook's own empty-guard stands between this and a blind oracle.
"$PYTHON" make_versioned_pe.py --version "$VER" --no-version --pad 70000 --out "$SIDE_CYSD"
expect_fail_with 'refusing to build a blind oracle' 'N1 versionless sidecar'
"$PYTHON" make_versioned_pe.py --version "$VER" --out "$SIDE_CYSD"

# N2 — 0.0.0.0 stamp: a readable-but-zero VERSIONINFO is an oracle that matches nothing real.
"$PYTHON" make_versioned_pe.py --version 0.0.0.0 --out "$SIDE_CYS"
expect_fail_with 'version resource regression' 'N2 zero-stamped sidecar'
"$PYTHON" make_versioned_pe.py --version "$VER" --out "$SIDE_CYS"

# N3 — missing sidecar: the /FileExists+!error pair must refuse before !getdllversion runs.
rm -f "$SIDE_CYS"
expect_fail_with 'sidecar source not found at compile time' 'N3 missing sidecar'
"$PYTHON" make_versioned_pe.py --version "$VER" --out "$SIDE_CYS"

# N4 — token presence in the REAL hook (not the copy): the release.yml / windows-build.yml
# build-log belts grep for these exact strings; wording drift silently disables them.
for tok in 'refusing to build a blind oracle' \
           'sidecar source not found at compile time' \
           'version resource regression'; do
  grep -qF "$tok" "$HOOK" || {
    echo "FAIL[N4]: contract token '$tok' not found verbatim in $HOOK — CI grep belts are blind" >&2
    exit 1
  }
done
echo "nsis-hook-compile: N4 OK (all 3 CONTRACT §5 tokens verbatim in the hook)"

# N5 — CONTRACT §6 taskkill census (the machine form of owner anchor ④). Outside the
# NSIS_HOOK_PREUNINSTALL macro there must be EXACTLY one taskkill — PREINSTALL's GUI-only
# '/F /IM cys-app.exe' — and no '/T' tree-kill on any line outside that macro (the GUI
# spawns cysd as a plain child: one stray tree-kill = every pane's agent dies. v0.14.27's
# CYS_SWAP_IN_PLACE kill fallback is the regression class this lane pins out).
# Contract and lane move in the same commit or not at all (NSIS-CONTRACT §6).
n5_anchors=$(grep -c 'NSIS_HOOK_PREUNINSTALL' "$HOOK" || true)
if [ "$n5_anchors" -ne 1 ]; then
  echo "FAIL[N5]: 'NSIS_HOOK_PREUNINSTALL' appears $n5_anchors times in the hook (want exactly 1) — the census window below is ambiguous; re-derive CONTRACT §6 before touching kills" >&2
  exit 1
fi
n5_total=$(grep -c 'taskkill' "$HOOK" || true)
n5_inside=$(awk '/NSIS_HOOK_PREUNINSTALL/,/!macroend/' "$HOOK" | grep -c 'taskkill' || true)
if [ "$n5_total" -ne 10 ] || [ "$n5_inside" -ne 9 ]; then
  echo "FAIL[N5]: taskkill census drifted — file total=$n5_total (contract: 10), inside PREUNINSTALL=$n5_inside (contract: 9)" >&2
  exit 1
fi
n5_outside=$(awk '/NSIS_HOOK_PREUNINSTALL/,/!macroend/{next} {print}' "$HOOK" | grep 'taskkill' || true)
if ! printf '%s' "$n5_outside" | grep -qF '/F /IM cys-app.exe'; then
  echo "FAIL[N5]: the single taskkill outside PREUNINSTALL is not the GUI-only '/F /IM cys-app.exe':" >&2
  printf '%s\n' "$n5_outside" >&2
  exit 1
fi
if printf '%s' "$n5_outside" | grep -q '/T'; then
  echo "FAIL[N5]: tree-kill (/T) outside PREUNINSTALL — the every-pane-dies class (owner anchor ④):" >&2
  printf '%s\n' "$n5_outside" >&2
  exit 1
fi
echo "nsis-hook-compile: N5 OK (taskkill census 9 inside PREUNINSTALL / 10 total; the 1 outside is GUI-only, no /T)"

# N6 — checklist anchors are real. docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md cites the
# hook by ANCHOR (macro/label/function names — raw line numbers rotted twice, R2/R3):
# every cited anchor must exist verbatim in the hook, and raw hook line-number citations
# must not come back. Renaming a hook anchor without updating the checklist reddens here.
CHECKLIST="../../../docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md"
if [ -f "$CHECKLIST" ]; then
  n6_missing=0
  for a in $(grep -oE 'CYS_[A-Z][A-Z_]+|cys_[a-z0-9]+_[a-z0-9]+|\.onInstFailed|\.onUserAbort|NSIS_HOOK_[A-Z]+' "$CHECKLIST" | sort -u); do
    if ! grep -qF "$a" "$HOOK"; then
      echo "FAIL[N6]: checklist cites hook anchor '$a' which does not exist in the hook — renamed/deleted anchor; update the checklist in the same commit" >&2
      n6_missing=1
    fi
  done
  [ "$n6_missing" -eq 0 ] || exit 1
  if grep -nE '(훅|hook)[^:0-9]{0,12}:[0-9]{2,3}' "$CHECKLIST" >&2; then
    echo "FAIL[N6]: raw hook line-number citation in the checklist (lines above) — cite anchors, not line numbers (they rot on every hook edit)" >&2
    exit 1
  fi
  echo "nsis-hook-compile: N6 OK (all checklist hook anchors exist; no raw line citations)"
else
  echo "FAIL[N6]: checklist not found at $CHECKLIST" >&2
  exit 1
fi

# ── re-verify the positive with restored inputs (negatives must leave no residue) ──
"$MAKENSIS" -V2 -WX -INPUTCHARSET UTF8 harness.nsi >/dev/null
echo "nsis-hook-compile: OK (hook compiled, macros inserted, getdllversion oracle exercised, 3 negative controls + token, census and anchor pins verified)"
