; nsis-hook compile harness — compiles the REAL src-tauri/nsis-hooks.nsh outside a Tauri build.
;
; Faithfully mirrors the include/define ORDER of the tauri-cli v2.11.4 template
; (crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi):
;   - the hook is !include'd FIRST (installer.nsi:34-35), BEFORE every template !define
;     (VERSION:42, MAINBINARYSRCPATH:53, UNINSTKEY:66). A hook that references template
;     symbols at top level would compile here exactly as it fails in the real build.
;   - the three hook macros are !insertmacro'd in template order (PREINSTALL/POSTINSTALL
;     inside Section Install, PREUNINSTALL inside Section Uninstall). An uninserted macro
;     body is never compiled by makensis, so insertion is what actually exercises the code.
;
; Inputs are prepared by run.sh into build/: a byte-identical copy of the hook and fake
; sidecar/main PEs carrying a real VS_VERSIONINFO resource (make_versioned_pe.py), so the
; hook's compile-time `!getdllversion /packed` oracle runs for real (no /noerrors).
Unicode true

; ── hook include — MUST stay ahead of the template defines below (template parity) ──
!include "build\nsis-hooks.nsh"

; ── template-supplied symbols (defined AFTER the hook include, as in installer.nsi) ──
!define PRODUCTNAME "cys"
!define VERSION "0.14.28"
!define MAINBINARYNAME "cys-app"
!define MAINBINARYSRCPATH "build\binaries\cys-app.exe"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"

Name "${PRODUCTNAME} hook harness"
OutFile "build\harness-setup.exe"
InstallDir "$TEMP\cys-hook-harness"
ShowInstDetails show
SetCompressor /SOLID lzma

; template-supplied function the POSTINSTALL hook Calls (installer.nsi:956).
; Stub: the harness only proves the hook COMPILES; it is never executed.
Function CreateOrUpdateDesktopShortcut
FunctionEnd

Section "Install"
  SetOutPath $INSTDIR

  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif

  ; (the real template extracts the main exe / resources / external binaries here)

  WriteUninstaller "$INSTDIR\uninstall.exe"

  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif
SectionEnd

Section "Uninstall"
  !ifmacrodef NSIS_HOOK_PREUNINSTALL
    !insertmacro NSIS_HOOK_PREUNINSTALL
  !endif
SectionEnd
