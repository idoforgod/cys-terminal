# WIN-MEASURE R3 FINAL — review handoff

Parent ca4e98bc6078e5fac8fe488677acbd07b53bd1e7; one local commit on work/0.14.53-win-measure. Read REPORT-R3.md for complete evidence, parent RED/GREEN, mutation provenance, every unbound channel and both F6 limitations. Native PowerShell/Windows/actionlint remain UNMEASURED; no CI results read.

Focus: driver-observation.mjs processObservation must reject lost/truncated/undrained/failed stdout before state or Seat-Check uses it. win-measure.ps1 Observe-ProcessOutput gates creation/screen/list/exit-list. vendor/lib.ps1 separates read/decoding/truncation metadata from product text. Unknown must survive baseline and old-restart finalization; complete observed failures remain failures.

Local104/104 tests and2distinct final mutants killed (one JavaScript behavior, one PowerShell source wiring). F3 completion is OUT of0.14.53 by head ruling686d0cafb221b493; all17inventory rows are explicitly UNMEASURED and remain fail-closed. PS5.1 parse-all unchanged is not a measured parse. No source signing/shipped-file change; input pins unchanged/empty. Stop after final-round handoff; master judges acceptance.
