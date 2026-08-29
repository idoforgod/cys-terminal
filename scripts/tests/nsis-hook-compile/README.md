# nsis-hook-compile — compile harness for src-tauri/nsis-hooks.nsh

Compiles the REAL hook outside a Tauri build, on any OS with `makensis` (v3.12 tested).

    bash scripts/tests/nsis-hook-compile/run.sh    # exit 0 = hook compiles AND guards fire

Why it exists: the hook is the single most dangerous file in this repo (a mistake bricks
every Windows user's CLI). An uninserted NSIS macro is never compiled, so this harness
mirrors the tauri-cli v2.11.4 template exactly — hook `!include`d BEFORE the template
`!define`s (installer.nsi:34-35 vs :41-66), then all three hook macros `!insertmacro`d —
and feeds the hook fake sidecar PEs carrying a real VS_VERSIONINFO resource so the
compile-time `!getdllversion /packed` oracle runs for real (no `/noerrors` anywhere).

Guarantees exercised (see `_work/win-installer-fix-20260829/NSIS-CONTRACT.md` §5·§7).
Each guard is proven by a **negative control inside run.sh** — a poisoned input that must
fail the compile WITH the frozen contract token (R2 round-2; a green positive compile
alone could not notice guard deletion — measured with an E1 mutation stripping all four
`!error` guards, which still exited 0 before the negatives existed):

  - N1 sidecar without readable VERSIONINFO -> compile FAILS with
    `refusing to build a blind oracle` (POSIX makensis silently defines EMPTY
    `!getdllversion` values instead of erroring — measured; only the hook's own
    empty-guard stands in the way)
  - N2 sidecar stamped 0.0.0.0             -> compile FAILS with
    `version resource regression`
  - N3 sidecar missing at compile time     -> compile FAILS with
    `sidecar source not found at compile time`
  - N4 all three §5 tokens exist VERBATIM in the real hook — the release.yml and
    windows-build.yml build-log belts grep these exact strings (token in log = fail),
    so wording drift in the hook would silently disable those belts; N4 pins the words
  - N5 CONTRACT §6 taskkill census (R3) — 9 kills inside NSIS_HOOK_PREUNINSTALL,
    10 in the file, and the single one outside is the GUI-only `/F /IM cys-app.exe`
    with no `/T`. This is the machine form of owner anchor ④ (no path may kill every
    pane's agent); a reintroduced kill fallback (v0.14.27 class) reddens here instead
    of passing every green gate
  - N6 checklist anchor pins (R3) — every hook anchor cited by
    `docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md` exists in the hook, and raw `:NNN`
    hook line-number citations are refused (they went stale twice, R2/R3; the checklist
    cites macro/label names instead)
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

Files: `harness.nsi` (template-parity stub), `make_versioned_pe.py` (fake PE generator;
`--no-version` emits the N1 negative-control PE), `run.sh` (runner; env: MAKENSIS, PYTHON,
CYS_HARNESS_VERSION). Build output in `build/` (gitignored).

CI wiring: `ci-branch.yml` job `nsis-hook-harness` (macos-latest, `brew install nsis`)
runs this same entry point on branch pushes/dispatch. The release lanes compile the hook
for real at build time (release.yml / windows-build.yml) and additionally grep the build
log for the §5 tokens — that belt is what N4 protects.
