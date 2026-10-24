# nsis-hook-exec — P1 executor (WP12 B1 r2)

`p1_exec.py` runs the **real** P1 macro text of `src-tauri/nsis-hooks.nsh` (the pre-swap run
check: `cys_probe_page`, `CYS_PRIOR_SCAN`, `CYS_PROBE_CLEAN`, `CYS_HANDOFF`, `CYS_PROBE_REFUSE`,
`CYS_REC_WRITE`, ...) with a small NSIS-subset interpreter, against simulated Windows outcomes.

```
python3 scripts/tests/nsis-hook-exec/p1_exec.py                 # exit 0 = every case passed
python3 scripts/tests/nsis-hook-exec/p1_exec.py --hook OLD.nsh  # run the same cases on other bytes
```

Exit codes: 0 all cases passed · 1 a case failed · 2 the executor could not run the text (an
instruction it does not implement, an undefined label or variable, an `!error` reached). Exit 2 is
never a pass. `scripts/tests/nsis-hook-compile/run.sh` runs it as its step 5, so the CI job
`nsis-hook-harness` (ci-branch.yml) runs it too.

## Why

The compile harness proves the hook compiles and its compile-time guards fire. The model
(`../nsis-hook-model/model.py`) mirrors the POSTINSTALL state machine by hand and pins the P1
structure. Neither executes the P1 macros. codex r1 (WP12-WINSAC-B1-0.14.53) found four defects
that pass both: a register overwriting its own input (B1-F3), an unbounded process query (B1-F2),
termination taken as proven without proof (B1-F1), and record writes that were never checked
(B1-F4). Each has cases here that fail on the old bytes (dcfd4bc4) and pass on the fix.

## What is real and what is simulated

| real (the hook's own text) | simulated (the operating system) |
|---|---|
| every P1 instruction as written: labels, jumps, registers, the error flag | files and folders: read-only files, running images that cannot be deleted, writes that fail after N bytes |
| `!insertmacro` / `!if` expansion with parameters substituted as text | child processes the hook starts: the probe-only worker, the three leaves, `tasklist` |
| instruction semantics from NSIS 3.13 `Source/exehead/exec.c` (StrCpy length/offset, 32-bit IntOp, FileRead keeping CR/LF, FileWrite setting the error flag, FileClose reporting nothing, IfErrors clearing the flag) | one model of every child process: it ends after N ms or never, may keep printing, may refuse TerminateProcess, may make WaitForSingleObject fail, may fail to start. The current hook reaches it through CreateProcessW / WaitForSingleObject(ms) / TerminateProcess / GetExitCodeProcess (System::Call, structs by address); older bytes reach it through nsExec, simulated as `Contrib/nsExec/nsexec.c` does it (same loop in NSIS 3.11 and 3.13): `/TIMEOUT` = time without output, the call returns only after the child ended - so a child that keeps printing, refuses termination or fails the wait makes such bytes run UNBOUNDED |
| `${StrLoc}` and `${GetOptions}` ported from their NSIS 3.13 sources | the clock (GetTickCount, Sleep), windows and processes for the hand-off, the console |

So a PASS is evidence about the hook's **logic** against those outcomes. It is not a Windows run:
real Smart App Control / App Control refusals, real process and termination timing and real dialogs stay UNMEASURED
until the `win-update-probe` job (windows-build.yml) runs on windows-latest.

## Cases

`X-R3-*` hard wall-clock bound of every child (worker, leaf, query) under hang / failed terminate / failed wait / endless output (codex r2 B1r2-F1) · `X-F1-*` worker outcome and leaf termination · `X-F2-*` hand-off bound and query failure ·
`X-F3-*` kept version through CRLF / LF / no newline · `X-F4-*` record persistence and what the
dialog and console name · `X-CTL-*` and the cases marked `control` must pass on old and new bytes
(they prove the executor runs the ordinary paths, so a RED elsewhere is about the finding).

If you change a P1 macro, run this file; if it reports an unsimulated instruction, extend the
executor in the same commit rather than deleting the case.
