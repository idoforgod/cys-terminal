# WIN-MEASURE R3-FIX (FINAL3/3) — local evidence report

Parent: ca4e98bc6078e5fac8fe488677acbd07b53bd1e7. Worktree: the isolated worktree; branch work/0.14.53-win-measure. Assignment: master ticket R3FIX-WINMEASURE-w12.txt, SHA256686d0cafb221b49322b3fb14fa34c5e79fcd28660265c09972440a1933f2dfe2. Head accepted the F2/W3 stream-loss finding and excluded F3 collector completion from0.14.53. One local commit; exact SHA/parent/source verification recorded in local-checks/r3/COMMIT.json and master's handoff. No assertion of delivery or acceptance.

## R3-1 — observed process exit is not observed stdout

At the parent, Invoke-Proc could return rc0 while stdout_complete=false, pipes_not_drained=true and empty/partial output. The state reducer accepted the exit record, compared the missing version output, and produced a known-false baseline. The early R1 branch then returned FAIL. Seat-Check similarly parsed missing new-surface/list output, or used an image-admission probe to turn response loss into a product failure.

The shared production `processObservation` in driver-observation.mjs:8 now separates two kinds of proof. A genuinely observed nonzero exit is a measured process failure without interpreting stdout. For rc0, successful collection is mandatory: stdout_complete=true, pipes_not_drained=false, no stdout_error, no stdout_truncated, and a string output. Missing flags/output, undrained pipe, read/decode error and truncation remain known=false/value=null with a reason. State version and identify/ping readers use that helper; a successful-process stream loss cannot be masked by the independent missing-image branch or flow through baseline=false. Complete malformed/empty output can still prove a product failure; missing output cannot.

Seat-Check invokes the same JavaScript adapter through `Observe-ProcessOutput` before parsing new-surface output and before reading screen/list/exit_list. Unknown returns before missing-PID/non-exit decisions. A separate image probe no longer substitutes for an unobserved response. Complete observed nonzero command exits remain failures, and healthy list evidence can still prove missing/non-exiting child. This PowerShell wiring is inspected and source-checked, not executed natively on this Mac.

Invoke-Proc now retains read/decoding errors and truncation in separate fields. Diagnostic text is no longer substituted for product stdout/stderr. Exact captured bytes remain separate; strict decoding rejects invalid/replacement-decoded text. Completeness stays false after read/decoding failure and text truncation.

## Parent RED, repaired GREEN, mutation evidence

- Parent production files were byte-compared to ca4e98bc before repair; hashes and commands are in local-checks/r3/parent-red-proof.json. Original test run:8 tests,6 stream-loss negatives failed at the named UNKNOWN/UNMEASURED assertions,2 healthy/measured-failure controls passed. The reproduction uses saved byte-identical parent modules, changes only test import paths, and again reports6failed/2passed. Raw logs and test snapshot are retained; no native/CI result is involved.
- Final local suite:104/104 Node tests pass. It includes the prior contracts and evidence gates, stream cases for versions/identify, all four Seat-Check process readers, and otherwise-complete evidence fixtures through finalizer/sealer. Baseline stream loss yields baseline=null and UNMEASURED; old-restart state/seat stream loss yields old_restart=null and UNMEASURED while the fixtures' evidence_complete/logs_complete remain true. Thus the result is caused by the missing observation, not an unrelated missing log.
- Input CLI controls remain2refusals/1positive: missing/hash-mismatched installer emits no permit/install sentinel; valid synthetic files authorize. This does not fill or query real input pins.
- Two final mutants KILLED: `stream-completeness-bypass` changes the production helper's unknown result to known(true), caught by the named baseline/output assertions; `seat-create-unknown-bypass` removes Seat-Check's return-on-unknown, caught by source integration assertion. The latter is explicitly a PowerShell source check, not native runtime proof. Initial and final executions are separately retained;2distinct final kills, not4independent mutants.
- YAML/trigger census and source integration checks pass. The new stream tests are included in the workflow's pre-install test command. The PS5.1 parse-all step remains byte-identical, with its hash recorded; it has NOT been run locally. Input pins stay byte-identical and empty. Vendor provenance stores original and adapted hashes.

Evidence directory: diag/local-checks/r3/. Key files: parent-red-proof.json, parent-red-reproduced.log, stream-red.log, green.log, mutants.json, mutants-initial.json, seat-parent-source-red.log, seat-source-green.log, wiring.log, workflow.log, input-controls.json, F6-host-evidence.json, PS51-step-preserved.json, SOURCE-SHA256.json, COMMIT.json, SHA256SUMS. Checksums bind the local records; they are not evidence of Windows execution.

## R3-2 — F3 scope disposition and every unbound channel

Head ruling: collector completion is OUT of0.14.53. No collector was silently added, no default marks a channel complete. Missing required channel, producer, interval, byte offsets or content hash continues to refuse a measured-success result. The inventory below and log-inventory.json explicitly name the status of each row. R1 requires15rows; R2 additionally requires the2old-restart rows. The two base-installer collectors exist but their final-product bindings remain empty; all other collectors remain unimplemented. Filling installer input pins alone does not make native R1/R2 ready.

| Channel | Status | Reason |
|---|---|---|
| daemon:file | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| daemon:stdout | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| daemon:stderr | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| pack:file | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| pack:stdout | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| pack:stderr | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app:file | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app-base:stdout | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app-base:stderr | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app-updated:stdout | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app-updated:stderr | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| installer-base:stdout | UNMEASURED | collector exists but final product source/sink binding is absent |
| installer-base:stderr | UNMEASURED | collector exists but final product source/sink binding is absent |
| installer-candidate:stdout | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| installer-candidate:stderr | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app-old-restart:stdout | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |
| app-old-restart:stderr | UNMEASURED | collector unimplemented; F3 completion OUT of 0.14.53 scope by head ruling |

Backlog recorded in worker TODO: F3 daemon/pack/app/updated-app/candidate-installer native collectors, OUT of0.14.53; backlog identifier awaits head assignment. No scope/bar relaxation is inferred from this report.

## R3-3 — the two unmet evidence items, separately disposed

| Requested evidence | Status | Exact reason / evidence actually available |
|---|---|---|
| Successful Windows PowerShell5.1 parse-all | **UNMEASURED** | Host is macOS; neither pwsh nor powershell is available on PATH. Windows PowerShell5.1 parsing was not executed. CI dispatch/result reading is prohibited. F6-host-evidence.json records host/tool availability; PS51-step-preserved.json records only that the future workflow step is unchanged, NOT a successful parse. actionlint is also unavailable/UNMEASURED; YAML parsing is a separate measured check. |
| Native PowerShell observation branches and serialization | **UNMEASURED** | No Windows process/SAC/Security event/PTY/CDP run is authorized or executed here. Local tests feed synthetic serialized Invoke-Proc/driver records through the production JavaScript adapter/finalizer; source assertions verify the PowerShell call sites. Neither replaces native PowerShell branch/serialization evidence. |

Other native Windows installation, SAC, audit/CDP/PTY, full channel collection and all CI outcomes remain UNMEASURED. No CI result was read. No local parse success or native success is claimed.

## Scope, provenance and stop

Only diagnostic harness/workflow tests/docs changed; no product build, product source, shipped Windows exe/DLL, signing/repacking, Defender/SAC allowlist, push/tag/release/upload or CI action. Existing input authorization, no-autostart observer seal, real updater exit parity, SAC restoration and result integrity gates remain. The report describes a local final-round repair and explicit limits, not Windows-unblock evidence. Master owns the r3 decision; no fourth round is inferred. Confidence High for source-bound/local results; unavailable native evidence is named UNMEASURED.
