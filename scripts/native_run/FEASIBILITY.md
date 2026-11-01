# Local feasibility slice (not execution authority)

`job.py authorize` binds the external authority's exact bytes to its digest,
GitHub push ref, attempt 1, diagnostic SHA, standard Windows host and UTC window.
`job.py execute --job feasibility` then starts an owned suspended/job-contained
worker. `setup.py` downloads only the frozen HTTPS receiver binary, verifies its
hash before execution, builds the diagnostic daemon and the local C# helper,
and emits observed binary identities. No receiver installer or unconfined
`--version` invocation is used.

The external JSON needs `execution_authorized: true`, `release_verdict: "NONE"`,
`mode: "feasibility"`, `attempt: 1`, full `product_sha` and `diagnostic_sha`,
`branch`, `host: "windows-latest"`, exact numeric `rust_toolchain`, and timezone
qualified `earliest_start_utc` / `latest_end_utc`. It must also bind a separately
reviewed `manifest: {"url": "https://...", "sha256": "..."}`. Before compilation,
the runtime recomputes product blob maps, diagnostic source hashes, controller
hashes and allowed deltas; each changed cfg(test) path needs its external
`review_approval` identifier. The manifest must identify exactly one installed
candidate as `installed_candidate: {"path": ".github/workflows/<name>.yml",
"sha256": "..."}`. Both generator and census accept that one digest-bound
addition and continue rejecting duplicate/altered/other matching workflows.
Use their `--installed-workflow-path` and `--installed-workflow-sha256` inputs
only for the eventual authorized installed tip; current preview stays uninstalled.
`probe_config` needs:

- `receiver_url`: exact reviewed HTTPS binary URL (redirects refused);
- `receiver_sha256` and `receiver_version`: externally frozen identity;
- `canary_literal_address`: the single reviewed public TCP address, port 443;
- `controller_hashes`: SHA-256 values for `boundary.py`, `boundary_host.py`,
  `controller.py`, and `protocol.py` at the diagnostic pin.

No such execution grant is supplied by this source. The generated manifest is
explicitly a draft, `freeze_ready=false`. The uninstalled workflow is JSON,
which is valid YAML; generating it does not put it under `.github/workflows`.
The preview branch is only an input to local trigger analysis, not an approved
remote branch or a permission to push.

The fixed 30-minute allocation includes setup from the first workflow step.
An external owned-job supervisor enforces phase caps without heartbeat resets
and reserves the final five minutes for collection. The workflow also has a
30-minute job limit. Slow setup or unavailable observations become NULL; there
are no repeated input attempts, reruns, extra jobs or adaptive sample sizes.
The build target and copied binaries are excluded from upload; their identities,
full build outputs, original raw observations and collection manifest remain.

`ProductReceiver` is the carrier shared with the forthcoming full R driver:
private daemon, ConPTY, zero-capability AppContainer, same launcher, recorder,
observer and controller. The real receiver is never replaced by a fake model.
The helper-origin calibration is in two separate no-Claude ConPTYs and is never
eligible as the real query. Job-member token snapshots are matched to total
created processes; an unobserved short-lived child leaves confinement NULL.
Native compatibility of these APIs has not been measured on macOS.

OPEN observations use only read-only frame RPCs before CLOSE. Editor exports
occur before OPEN or after CLOSE; each capture publishes length/hash metadata,
waits for an acknowledgement while the observer verifies the helper process,
then supplies an observed exit and a fresh application render. This tests a
candidate temporal discriminator; matching frames or missing receipts mean
NULL. The rendered screen is not substituted for the exact-byte export oracle.
The real-query origin is deliberately NULL pending a deciding discriminator;
calibration alone cannot establish that one exists. Known preservation or
unexpected-submission failures remain visible alongside NULL.

Submission controls use two fresh receivers, a fixed 10-second passive window,
native input receipts, and session-bound raw application records. A missing
positive event is NULL, not a failed submission inferred from silence. An
unsubmitted control with a user event or multiple positive events is retained
as an observed failure. No backend is started to make this control pass.

Current local validation is Python contract controls, targeted Rust recorder
execution and a Windows-target Rust typecheck. The typecheck uses a stand-in C
compiler for cross-build dependencies; it is not a native build or execution.
The C# helper has not been compiled locally (no C# compiler on this host).

This is the first slice only. The main I300/L60/D280/R80/K20 driver and final
full battery are pending. The main plan stays 14 jobs, max 10 parallel, 1,190
runner-minutes. `job.py` refuses an incomplete main driver. No native hard gate
or release verdict is closed by this implementation.

Review delta: auth-free startup remains NULL until all four perturbation controls
complete successfully. Every receiver cleanup publishes a confinement value,
including explicit NULL for a missing finish receipt. Network feasibility is
assigned only after all recorded confinement obligations are combined. Valid
submission events are accumulated independently from malformed/missing sibling
streams; completeness becomes NULL without erasing an already observed failure.
