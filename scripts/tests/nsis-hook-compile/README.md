# nsis-hook-compile — compile harness for src-tauri/nsis-hooks.nsh

Compiles the REAL hook outside a Tauri build, on any OS with `makensis` (v3.12 tested).

    bash scripts/tests/nsis-hook-compile/run.sh    # exit 0 = hook compiles

Why it exists: the hook is the single most dangerous file in this repo (a mistake bricks
every Windows user's CLI). An uninserted NSIS macro is never compiled, so this harness
mirrors the tauri-cli v2.11.4 template exactly — hook `!include`d BEFORE the template
`!define`s (installer.nsi:34-35 vs :41-66), then all three hook macros `!insertmacro`d —
and feeds the hook fake sidecar PEs carrying a real VS_VERSIONINFO resource so the
compile-time `!getdllversion /packed` oracle runs for real (no `/noerrors` anywhere).

Guarantees exercised (see `_work/win-installer-fix-20260829/NSIS-CONTRACT.md`):
  - sidecar missing at compile time      -> compile FAILS (top-level `!if ! /FileExists`
    + `!error`)
  - sidecar without readable VERSIONINFO -> compile FAILS (blind-oracle guards: `!error`
    on empty or 0.0.0.0 `!getdllversion` output; POSIX makensis silently defines EMPTY
    values instead of erroring - measured)
  - `!getdllversion` fed an unexpanded template symbol (e.g. a top-level
    `${MAINBINARYSRCPATH}`) -> compile FAILS (the unexpanded literal is not a readable
    file - measured)
  - all three hook macros really inserted, compiled with `-WX` (any warning is fatal)

  NOT caught (measured): a bare `!define` that merely captures a template symbol at hook
  top level (e.g. `!define X "${VERSION}"` before the template defines exist) compiles
  EXIT 0 with zero warnings - the NSIS preprocessor silently stores the unexpanded
  literal `${VERSION}`. The include-order trap fires ONLY where such a literal reaches a
  hard-failing consumer (`!getdllversion`, `/FileExists` + `!error`). Keeping template
  symbols out of the hook top level (IMPL-SPEC W4-A) rests on review discipline plus the
  final-file audit, not on this harness alone.

Files: `harness.nsi` (template-parity stub), `make_versioned_pe.py` (fake PE generator),
`run.sh` (runner; env: MAKENSIS, PYTHON, CYS_HARNESS_VERSION). Build output in `build/`
(gitignored). CI uses the same entry point; on Windows runners point MAKENSIS at the
installed NSIS 3.12+.
