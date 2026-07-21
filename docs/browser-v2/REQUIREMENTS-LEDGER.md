# Browser v2 requirement ledger

Status: active implementation ledger
Baseline: `v0.13.1` (`06e6a3ac63a54fadd7427a950f2f5d571df7f9bd`)
Product authority: `CYS_TERMINAL_BROWSER_V2_PRD_2026-07-21.md`
Implementation authority: `CYS_TERMINAL_BROWSER_V2_통합설계안_3R.md`

The PRD owns goals, scope, user-visible acceptance criteria, and invariants. The 3R design owns newer implementation decisions. A row marked deferred remains release-blocking unless the PRD explicitly places it out of scope.

## Status vocabulary

- `baseline`: behavior existed at v0.13.1 and is protected by a named test.
- `slice-green`: implemented in the current Browser v2 vertical slice with a public-interface test.
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
| B2-G05 | Browser startup is lazy: boot and layout restore perform state lookup only; explicit globe/reconnect may start it. | UI → Tauri runtime manager | restore/boot process census | baseline UI behavior; runtime-manager migration deferred-P0 |
| B2-G06 | Official builds do not require user Bun, `node_modules`, or system Chrome. | signed browser runtime bundle | clean HOME/PATH packaged E2E | deferred-P0; ADR-0002 |
| B2-G07 | GUI, browserd protocol/build, and Chromium build form one compatibility unit. | runtime manifest + compatibility gate | skew matrix | deferred-P0 |
| B2-G08 | Browser failure cannot stop cysd, PTYs, organization boot, or terminal panes. | lifecycle isolation | failure-injection packaged E2E | release-gate |
| B2-G09 | The default pane is the shared `default` agent context and is labelled as shared; human profile is never cast. | session/control + GUI | control/profile integration tests | baseline server gate; label E2E deferred-P0 |
| B2-G10 | Human control blocks agent mutations until explicit handoff or lease expiry. | context arbiter | concurrent human/agent integration | baseline; generation-safe lease audit deferred-P0 |

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
| B2-P10 | Each client has at most one outstanding render and paints before sending `ack(fid)`. | `cysjavis-pack/browserd/server.ts` frameRecipients + `cysjavis-pack/browserd/cast.ts` CAST_APP_HTML | slow-client browser E2E | server rejects stale/duplicate/non-recipient ack; per-client slow-client E2E remains deferred-P0 |
| B2-P11 | One-time embed credentials are TTL-bound to runtime instance/context/embed generation and cannot be replayed. | `CastEmbedTicketRegistry`, app GET issue → WS consume | unit + real WS replay/legacy-bypass integration | slice-green |

## Runtime, persistence, and resource lifecycle

| ID | Requirement / invariant | Evidence | Status |
|---|---|---|---|
| B2-R01 | Tauri `BrowserRuntimeManager` is the sole production lifecycle owner; Python remains a compatibility adapter and never independently races a spawn. | lifecycle concurrency tests | deferred-P0 |
| B2-R02 | Production uses verified absolute paths and a signed runtime manifest; external Bun fallback is development-only. | clean-machine package inspection | deferred-P0; ADR-0002 |
| B2-R03 | Layout persistence never stores token, port, ticket, or live generation; restore reconstructs a pending descriptor. | `webpane.test.ts` | slice-green |
| B2-R04 | Retry is single-flight, bounded by attempts and wall time, cancellable on pane close, and never runs from passive restore. | `src/browser_runtime/lifecycle.rs` + unit tests in `src/browser_runtime/mod.rs` | typed policy contract green; Tauri single-flight/pane-close integration remains deferred-P0 |
| B2-R05 | Reconnect grace precedes screencast/CDP release; final idle lease reclaims browser resources without killing cysd/PTYs. | process/CDP census | slice-green grace; idle packaged evidence deferred-P0 |
| B2-R06 | Runtime update is stage → signature/hash verify → atomic select → health → commit/rollback journal. | `src/browser_runtime/lifecycle.rs` + unit tests in `src/browser_runtime/mod.rs` | typed journal contract green; filesystem/update integration remains deferred-P0 |
| B2-R07 | Rollback never mutates a signed app bundle in place or falls back to an unqualified external runtime. | signed rollback drill | deferred-P0; ADR-0004 |

## Freeze zones and release integrity

| ID | Frozen contract / release gate | Evidence | Status |
|---|---|---|---|
| B2-F01 | First ordinary pane is a login shell and is available before browser or master-team bootstrap. | pre/post packaged onboarding E2E | freeze |
| B2-F02 | Runtime PATH ordering, bundled Node/npm/npx/Python/Git, Unix `~/.local/bin`, and fresh Windows user PATH remain unchanged. | `cargo test --lib`; byte-equivalent PATH snapshot | freeze |
| B2-F03 | Claude installed into `~/.local/bin` is found in the same pane without restart; Codex/Antigravity adapters remain discoverable. | clean HOME installation E2E | freeze; release-gate |
| B2-F04 | User-owned agent configuration, bare-shell reinject skip, and role-declaration bootstrap order remain unchanged. | pack/boot regression suites | freeze |
| B2-F05 | Browser code never changes process-global PATH, PTY creation, cysd boot, or organization topology. | `git diff -- src src-tauri` for this packet + regression suite | freeze |
| B2-L01 | Two notarized/stapled macOS DMGs, Windows installer, Windows ZIP, complete SHA256SUMS, and one promoted download manifest represent the same version. | artifact manifest + remote verification | release-gate |
| B2-L02 | Windows downloads retain the Defender guidance section and remote verification asserts its presence. | local and remote content grep | release-gate |
| B2-L03 | Release requires independent adversarial review and dependency/packaging audit; producer self-score is not acceptance. | reviewer 1/2 evidence bundle | release-gate |

## Current packet boundary

This packet may change browserd cast/server code, web-pane browser wiring, tests, and Browser v2 documentation. It must not implement the Tauri runtime sidecar or edit release workflows/assets. It must not change `src/`, `src-tauri/`, `cysjavis-pack/bin/javis_browser.py`, PATH/PTY/onboarding/pack bootstrap logic, or user-owned configuration semantics.
