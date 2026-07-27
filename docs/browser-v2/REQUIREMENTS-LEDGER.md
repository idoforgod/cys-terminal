# Browser v2 requirement ledger

Status: active implementation ledger
Baseline: `v0.13.1` (`06e6a3ac63a54fadd7427a950f2f5d571df7f9bd`)
Product authority: `CYS_TERMINAL_BROWSER_V2_PRD_2026-07-21.md`
Implementation authority: `CYS_TERMINAL_BROWSER_V2_통합설계안_3R.md`

The PRD owns goals, scope, user-visible acceptance criteria, and invariants. The 3R design owns newer implementation decisions. A row marked deferred remains release-blocking unless the PRD explicitly places it out of scope.

## Status vocabulary

- `baseline`: behavior existed at v0.13.1 and is protected by a named test.
- `slice-green`: implemented in the current Browser v2 vertical slice with a public-interface test.
- `implementation-under-re-audit`: source implementation exists, but independent re-audit has not accepted it.
- `deferred-P0`: required before Browser v2 release; not implemented by the current packet.
- `release-gate`: evidence must come from packaged or remote artifacts, not a unit test.
- `freeze`: byte/behavior preservation contract; Browser work may not alter the owning subsystem.

## Product and architecture traceability

| ID | Requirement / invariant | Owning boundary | Evidence or planned gate | Status |
|---|---|---|---|---|
| B2-G01 | Browser lives in the current workspace pane and retains the single cast address bar. | `ui/src/main.ts`, cast HTML | packaged GUI E2E | baseline; packaged evidence deferred-P0 |
| B2-G02 | Internet content executes only in isolated Chromium; Tauri receives pixels and sends validated input. | browserd CDP/cast + sandbox iframe | browserd integration + packaged security E2E | baseline |
| B2-G03 | `SHELL_READY`, painted `FRAME_READY`, `LIVE`, `DEGRADED`, `FAILED`, and `CLOSED` are distinct; spawn/load is never success. | protocol/state machine | state-transition unit + child protocol test; packaged first-frame E2E | slice-green; packaged paint evidence remains release-gate |
| B2-G04 | Only HTTP/HTTPS navigation is permitted for product browsing. | browserd navigation policy | initial, redirect, popup, external-protocol, download tests | slice-green; internal `about:blank` is private and URL-less |
| B2-G05 | Browser startup is lazy: boot and layout restore perform state/embed lookup only; explicit globe/reconnect may start it. | UI → Tauri adapter → lazy cysd authority broker | `passive_probe_never_launches_or_writes`, authority broker single-flight tests; restore/boot process census | slice-green; packaged census remains release-gate |
| B2-G06 | Official builds do not require user Bun, `node_modules`, or system Chrome. | signed browser runtime bundle | strict absolute Chromium-path test, executable/tree mutation tests; clean-machine packaged E2E | runtime code slice-green; actual target bundle evidence release-gate; ADR-0002 |
| B2-G07 | GUI, browserd protocol/build, and Chromium build form one compatibility unit. | runtime manifest + compatibility gate | manifest identity tests + packaged skew matrix | slice-green contract; packaged skew matrix release-gate |
| B2-G08 | Browser failure cannot stop cysd, PTYs, organization boot, or terminal panes. | lifecycle isolation | bounded one-at-a-time Browser worker, immediate backpressure, timeout cancellation and real-child reap tests; failure-injection packaged E2E | implementation-under-re-audit; packaged cysd/PTY/process evidence remains release-gate |
| B2-G09 | The default pane is the shared `default` agent context and is labelled as shared; human profile is never cast. | session/control + GUI | control/profile integration tests + packaged label E2E | slice-green; packaged label evidence release-gate |
| B2-G10 | Human control blocks agent mutations until explicit handoff or lease expiry. | context arbiter | concurrent human/agent integration | slice-green including pane/session/generation lease ownership |
| B2-G11 | Runtime start from the GUI requires a registered signed app identity and a trusted, short-lived native activation; UI assertions and iframe messages are not authority. | kernel peer PID → signed/canonical GUI registration → lexical Tauri initialization capability → one-time cysd receipt | basename-spoof/PID-incarnation/replay tests, native window/TTL/single-consume tests, no-postMessage-ensure UI contract | implementation-under-re-audit; packaged macOS designated-requirement, Windows Authenticode signer parity and physical click-to-start evidence remain release-gates |

## Cast Protocol v2

| ID | Requirement / invariant | Public contract | Evidence | Status |
|---|---|---|---|---|
| B2-P01 | Supported protocol version is explicit and unsupported embed descriptors fail closed. | `CAST_PROTOCOL_VERSION`, cast app route | `webpane.test.ts`, `cast.test.ts` | slice-green |
| B2-P02 | Every iframe load has a positive embed generation; stale-generation parent messages are rejected. | `castAppUrl`, `acceptsCastMessage` | `webpane.test.ts` | slice-green |
| B2-P03 | Parent accepts a cast message only when source, exact iframe origin, protocol, generation, and embed ticket all match. | `acceptsCastMessage` | `webpane.test.ts` | slice-green |
| B2-P04 | Cast parent CSP is route-local and contains one exact platform/build origin; cast/RPC/audit policies are not globally relaxed. | `resolveCastParentOrigin`, `castContentSecurityPolicy` | `cast.test.ts` + route integration | slice-green; packaged origin measurement remains release-gate |
| B2-P05 | Parent and child use exact `targetOrigin`; wildcard `postMessage` and wildcard `frame-ancestors` are forbidden. | cast response body + UI iframe messaging | lexical gate + browserd/UI tests | slice-green |
| B2-P06 | Frame IDs are browserd-monotonic and never use CDP `sessionId` as a public dedup key. | server frame envelope | existing stream integration | baseline |
| B2-P07 | A fid causes at most one CDP ack; unacknowledged backlog is bounded and latest-frame-wins. | `LatestFrameFlow` | `cast.test.ts`; stream integration | slice-green |
| B2-P08 | A dropped, evicted, detached, or rendered frame is eventually acked so CDP cannot stall. | `LatestFrameFlow` integration | stream/resize/rebind integration | slice-green; full browserd integration green |
| B2-P09 | A transient zero-client interval keeps the cast session through a bounded reconnect grace. | `reconnectGraceDecision` + server timer | unit + disconnect/reconnect integration | slice-green; timed integration green |
| B2-P10 | Each client has at most one outstanding render and paints before sending `ack(fid)`. | `cysjavis-pack/browserd/server.ts` client frame receipts + `cysjavis-pack/browserd/cast.ts` CAST_APP_HTML | real-Chromium slow-client WS integration: one outstanding render, latest-frame catch-up, independent fast-client continuity | slice-green; packaged slow-client GUI evidence remains release-gate |
| B2-P11 | One-time embed credentials are broker-pre-registered over an inherited-session-key-authenticated private channel, TTL-bound to runtime/context/embed generation/pane/origin, opened by exactly one app GET and consumed by exactly one WS. No control or cast token is returned in `EmbedDescriptor`. | broker `prepare_embed`, `CastEmbedTicketRegistry` | request-MAC tamper tests, exact preregistration/header/DTO test, GET/WS replay integration | slice-green; packaged exact-origin E2E release-gate |
| B2-P12 | Human (pane) viewports narrower than `FIT_MIN_WIDTH` render at desktop width (fit-to-width) so fixed-width sites are not clipped; agent-pinned viewports stay literal. Aspect ratio is preserved so `mapInput` letterbox inversion stays exact. | `fitHumanViewport`, `FIT_MIN_WIDTH` (cast.ts is canonical); server WS `viewport` human path | `phase3-unit.test.ts` fitHumanViewport cases (narrow-expand, boundary, area-cap, degenerate) | slice-green; live pane-clip GUI evidence remains release-gate |
| B2-P13 | Cast frames render sharp on the pane: the canvas backing store is sized `CSS × devicePixelRatio` so Retina panes do not upscale a CSS-px bitmap; and the jpeg quality ladder guarantees a 60 floor for fit-to-width viewports (≈1.9MP) instead of dropping to 45. Mouse coordinates stay CSS-based (`offsetX`/`canvas.clientWidth`) so `mapInput` inversion is unaffected by the DPR backing. Capture-side DSF is separate (4-S-9 2차). | `fitCanvas` (DPR backing) + `jpegQualityFor` (≤`VIEWPORT_MAX_AREA`→60) in cast.ts CAST_APP_HTML | `phase3-unit.test.ts`: `jpegQualityFor(1080,1747)=60`, `1280×800=75` regression, `2000×1500=45` safety-net; CAST_APP_HTML `devicePixelRatio` wiring grep | slice-green; live Retina sharpness GUI evidence remains release-gate |

## Runtime, persistence, and resource lifecycle

| ID | Requirement / invariant | Evidence | Status |
|---|---|---|---|
| B2-R01 | The lazy cysd authority broker is the sole public lifecycle owner; its private supervisor owns process/lock state. Disk JSON alone is never authority: a live runtime must match the broker-retained session, state MAC, exact PID incarnation and authenticated health response. The authenticated endpoint is returned as one immutable snapshot; dead sessions/keys are pruned from a hard-bounded registry. Tauri, `cys`, and Python are adapters and never independently race a spawn. | disk-state denial, endpoint replacement-race regression, state-MAC/PID-incarnation validation, crash-loop cleanup, restart single-flight and supervisor-lock tests | implementation-under-re-audit; packaged process/key-lifecycle census remains release-gate; ADR-0002 |
| B2-R02 | Production uses verified absolute paths, a hash of the complete Chromium tree, and minisign-verified runtime attestation/policy bytes rooted in the compiled active/non-revoked keyring. External Bun/Chrome fallback is development-only. | strict path/tree-mutation and minisign trust-root tests; `release/browser-runtime-sources.json` pins exact Rust/Bun/Playwright/headless-shell inputs; atomic stage rejects digest/size/path/link/type/architecture/license drift; platform code signing precedes final metadata hashes | implementation-under-re-audit; both notarized macOS target bundles, Windows Authenticode/NSIS and installed-artifact proof remain release-gates; ADR-0002 |
| B2-R03 | Layout persistence never stores token, port, ticket, or live generation; restore reconstructs a pending descriptor. | `webpane.test.ts` | slice-green |
| B2-R04 | Retry is single-flight, bounded by attempts and wall time, cancellable on pane close, and never runs from passive restore. | broker mutex + `src/browser_runtime/lifecycle.rs` + UI generation/dispose guards | typed policy/single-flight green; pane-close packaged integration release-gate |
| B2-R05 | Reconnect grace precedes screencast/CDP release; final idle lease reclaims browser resources without killing cysd/PTYs. | process/CDP census | slice-green grace; idle packaged evidence deferred-P0 |
| B2-R06 | Runtime update is stage → signature/hash verify → atomic select → health → commit/rollback journal. | compiled `src/browser_runtime/lifecycle.rs` state machine and crash-recoverable selector; macOS filesystem tests + Windows cross-check | slice-green journal and atomic selection store; broker/package multi-unit wiring and signed update integration remain deferred-P0 |
| B2-R07 | Rollback never mutates a signed app bundle in place or falls back to an unqualified external runtime. | signed rollback drill | deferred-P0; ADR-0004 |
| B2-R08 | The public `cys browser` compatibility command targets the shared in-pane context; headful-only development verbs never report production success without a visible window. | CLI/adapter tests + observe runbook | slice-green |

## Diagnostic contract (v0.13.21 browser quality overhaul)

Authority: `docs/DIAGNOSTIC-CONTRACT.md` (three rules, full code registry, self-inflicted sign
filter, filter-then-last extraction, 26-character code ceiling). Rows below register the codes this
branch created or newly exposed; the contract document is the single source of truth for their
meaning, trigger condition, `file:line` and user action.

| ID | Requirement / invariant | Public contract | Evidence | Status |
|---|---|---|---|---|
| B2-D01 | Every fail-closed refusal carries a machine-readable `error.code` end to end (broker → NDJSON → `rpc_oneshot_full` → `RpcFailure::banner()` → GUI pane banner). `rpc_full` is never called directly. | `BrokerFailure{code,message}`, `BROWSER_DISABLED_SAFE [<CODE>]: <message>` | `unwrap_rpc_result_preserves_code_and_message`, `gui_registration_lost_matches_code_first_then_marker` | slice-green; two early returns in `browserd_state` still emit bare `{alive:false}` (debt D1) |
| B2-D02 | The cause appears within the first 120 characters of the message (banner truncation budget); the sign leads and prose/`cys-browserd: ` prefixes are log-only. | `supervisor_death_message`, `normalize_sign`, `exit_annotation`; `castFailureReason` maxLen 120 (cast path 200) | `supervisor_sign_leads_the_message_and_fits_the_banner` (≤120 assertion), `banner_code_survives_ui_truncation_budget` | slice-green |
| B2-D03 | Contract strings that cross a process or crate boundary are pinned by `include_str!` tripwires in positive/negative-control pairs; a bare `contains` without a negative control is not acceptance. | `broker_registration_lost_marker_matches_broker_source`, `gui_branch_codes_exist_in_broker_source`, `banner_bracket_convention_matches_ui_fixture`, `banner_code_survives_ui_truncation_budget` | four compile-time tripwires in `src-tauri/src/main.rs` | slice-green |
| B2-D04 | New or renamed diagnostic codes are at most 26 characters, so the message keeps a 68-character budget inside the banner. | `docs/DIAGNOSTIC-CONTRACT.md` §2 | `every_error_code_literal_in_this_module_fits_the_26_char_cap` source-scan tripwire in both broker modules; `assert_banner_budget` asserts code ≤26 and banner ≤120 on actually produced failures | slice-green; automated in both directions (literal scan + produced value) |
| B2-D05 | Supervisor readiness failures are classified rather than collapsed: `SUPERVISOR_EXIT_PRE_READY` (no readiness frame), `SUPERVISOR_READY_OVERFLOW` (>64 KiB), `SUPERVISOR_READY_TIMEOUT` (25 s). `RUNTIME_START_FAILED` is retained, not retired, for the remaining eleven production sites. | `runtime_launcher.rs` launch/readiness paths | `silent_immediate_death_is_classified_with_its_exit_status` | slice-green for `EXIT_PRE_READY`; overflow and timeout branches have no unit test (debt D4) |
| B2-D06 | Sounds caused by our own pipe closure are not signs: the self-inflicted filter list is a contract constant, and extraction is filter-then-last over complete lines in an 8 KiB ring with a bounded 200 ms EOF wait. | `SELF_INFLICTED_SIGNS`, `StderrTap::last_sign` / `death_sign` | `self_inflicted_liveness_lines_are_not_mistaken_for_a_sign`, `oversized_stderr_keeps_the_last_complete_line`, `incomplete_trailing_line_is_not_reported_as_a_sign`, `tap_eof_synchronisation_collects_the_sign_every_time` | slice-green |
| B2-D07 | A department daemon reports the truth instead of failing opaquely: launch verbs (`ensure`, `prepare_embed`, `operation`) return `RUNTIME_UNSUPPORTED` with zero spawns, while `probe` answers with the tagged `BrokerStatus::Unsupported { reason }` variant and the GUI registration handshake survives. | `DEPT_BLOCKED_RUNTIME_OPERATIONS`, `dept_unsupported_status()`, `#[serde(tag="status")]` | `dept_daemon_blocks_browser_launch_verbs_but_keeps_probe_and_handshake` | slice-green; reason string is still Korean prose against the repo's English message convention (debt D2) |
| B2-D08 | Ownership refusal distinguishes self from foreign: `RUNTIME_SELF_UNVALIDATED` (this daemon recently owned it, revalidation failed) precedes `RUNTIME_FOREIGN_OWNER` (live genuine supervisor, canonical absolute-path equality). PID reuse is excluded by a signed `started_at ↔ start_time` window of `[-2s, +25s]`, not a symmetric ±2s: the supervisor stamps `started_at` only after the engine finishes booting, so real drift is seconds to tens of seconds and the process must have started before the stamp, within the broker readiness budget. Neither verdict ever kills. | `classify_disk_state_owner`, `classify_live_owner` | `our_own_unvalidated_runtime_is_not_reported_as_a_foreign_owner`, `basename_only_cys_browserd_impostor_cannot_block_a_launch`, `stale_state_with_a_dead_supervisor_still_launches`, plus boundary tests pinning the window with realistic `+9s`/`+12s` fixtures | slice-green as of `5215463`; the earlier ±2s formulation never opened in production and is retired |

## Freeze zones and release integrity

| ID | Frozen contract / release gate | Evidence | Status |
|---|---|---|---|
| B2-F01 | First ordinary pane is a login shell and is available before browser or master-team bootstrap. | pre/post packaged onboarding E2E | freeze |
| B2-F02 | Runtime PATH ordering, bundled Node/npm/npx/Python/Git, Unix `~/.local/bin`, and fresh Windows user PATH remain unchanged. | `cargo test --lib`; byte-equivalent PATH snapshot | freeze |
| B2-F03 | Claude installed into `~/.local/bin` is found in the same pane without restart; Codex/Antigravity adapters remain discoverable. | clean HOME installation E2E | freeze; release-gate |
| B2-F04 | User-owned agent configuration, bare-shell reinject skip, and role-declaration bootstrap order remain unchanged. | pack/boot regression suites | freeze |
| B2-F05 | Browser code never changes process-global PATH, PTY creation, cysd boot, or organization topology. | freeze-zone diff audit (`src/bin/cysd/governance.rs` must be byte-identical to HEAD), PATH/PTY/boot regression suites | freeze |
| B2-L01 | Two notarized/stapled macOS DMGs, Windows installer, Windows ZIP, complete SHA256SUMS, and one promoted download manifest represent the same version. | artifact manifest + remote verification | release-gate |
| B2-L02 | Windows downloads retain the Defender guidance section and remote verification asserts its presence. | local and remote content grep | release-gate |
| B2-L03 | Release requires independent adversarial review and dependency/packaging audit; producer self-score is not acceptance. | reviewer 1/2 evidence bundle | release-gate |

## Current implementation boundary

The earlier protocol-only packet boundary is superseded by the owner-authorized Browser v2 implementation scope. Browser runtime, Tauri adapter, UI, browserd and release-contract files may change together when traced in this ledger. This wording does not assert release completion: packaged first-frame/slow-client/idle/update/rollback tests, qualified target resources, signed/notarized artifacts and remote promotion remain explicit blockers above. PATH synthesis, PTY creation, ordinary-pane ordering, onboarding, pack reinject, organization bootstrap and user-owned configuration remain freeze zones: Browser work may test them but must not change their semantics.

The accepted simulation contracts are versioned in `docs/browser-v2/SIMULATION-CONTRACTS-2026-07-21.md`. A `slice-green` status never substitutes for packaged, signed, notarized, OS-specific or remote release evidence.

## Round 2 verification snapshot

The following source-level evidence was reproduced on 2026-07-21. Counts are test cases, not assertions.

| Command / gate | Result |
|---|---|
| browserd `bun run test`, serial canonical run 1 and run 2 | 93/93 each, exit 0 |
| UI `bun test` + `bun run build` | 178/178 + production bundle, exit 0 |
| `cargo test -p cys-browserd --lib` | 3/3, exit 0 |
| cysd Browser cancellation/runtime/signer focused suites | 4/4 + 6/6 + 1/1, exit 0 |
| cysd serial regression | 465/465, exit 0; pre-existing host `hwmon::` tests and `governance::tests::corrupt_topology_isolated_not_silently_empty` (reads the real user generation root) were explicitly excluded |
| root library / `cys` / `cys-app` | prior snapshot 150/150 + 88/88; current `cys-app` 32/32, exit 0 |
| release-script canonical discovery | 46/46, exit 0 |
| Python Browser adapter | 4/4, exit 0 |
| Browser negative gate | all assertions passed, exit 0 |
| eight primary Browser Rust modules (`supervisor`, Tauri, cysd broker/launcher/main, runtime lifecycle/mod/path) `rustfmt --check`; Python/Bash/PowerShell syntax; `git diff --check` | exit 0 |
| full tracked-tree secret scan | 731 files, clean, exit 0 |
| `src/bin/cysd/governance.rs` freeze check | HEAD and working-tree SHA-256 both `ca932b0abb344ec40267f1045102857c866727b5c1914e5604e106f591bc09b3` |

This snapshot is not packaged-release evidence. `src-tauri/resources/browser-runtime/` still requires qualified per-target runtime bytes and the external `release-production` GitHub Environment must exist with its protection rules and credentials before candidate/publish workflows can pass.

## 2026-07-22 continuation snapshot

| Command / gate | Result |
|---|---|
| browserd `bun run test`, including per-client slow-frame flow | 94/94, exit 0; slow client remains at one unacknowledged frame, catches up to the latest frame after ack, and does not stall a fast client |
| root library `cargo test --lib` | 154/154, exit 0; Browser update journal is compiled, rejects skipped/post-terminal transitions, rolls back interrupted selection with the exact previous generation, and commits only after health |
| Windows library cross-check `cargo check --locked --lib --target x86_64-pc-windows-gnu` | exit 0 with the rustup toolchain; `MoveFileExW` atomic replacement path compiles |
| local arm64 Browser Runtime stage/sign/smoke | exit 0; pinned inputs verified, six Mach-O files Developer ID signed and verified, signed engine launched the bundled absolute Chromium path and completed HTTP open + snapshot; see `LOCAL-RUNTIME-SMOKE-2026-07-22.md` |
| Browser Runtime metadata unit tests | 3/3, exit 0; final signed-tree rehash and trust-root contracts remain covered without materializing a release secret |
| changed Browser Rust modules `rustfmt --check` | exit 0 |
| `git diff --check` | exit 0 |

This closes the source-level B2-P10 gap and compiles/tests the B2-R06 journal and
selection-store slice. The local arm64 package smoke reduces staging risk but is
not notarized, installed, GUI, multi-target, or remote release evidence; those
release gates remain open.
