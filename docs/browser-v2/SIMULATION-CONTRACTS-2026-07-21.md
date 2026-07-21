# Browser v2 three-round simulation contracts

Status: accepted implementation and review contract
Product authority: `CYS_TERMINAL_BROWSER_V2_PRD_2026-07-21.md`
Implementation authority: `CYS_TERMINAL_BROWSER_V2_통합설계안_3R.md` plus accepted ADRs

The simulation did not change the product goal, scope, non-goals or bootstrap invariants. It converted hidden implementation assumptions into twelve release-blocking contracts. Newer implementation detail follows the 3R design and ADRs; requirements omitted by the 3R design remain inherited from the PRD.

## Round 1 — normal user flow

Simulation: clean installation with no user Bun, Chrome or `node_modules`; ordinary pane first; globe click; cold runtime start; authenticated cast shell; painted first frame; navigation/input; transient reconnect; close.

Findings: lifecycle ownership was ambiguous, UI received an overpowered credential shape, iframe load could be mistaken for readiness, and WS-zero cleanup conflicted with reconnect grace. The accepted owner is now the lazy cysd authority broker with a private supervisor. Tauri/CLI/Python are adapters. `FRAME_READY` requires decode and paint before ack. WS zero begins grace rather than immediate session destruction.

## Round 2 — adversarial concurrency and fault injection

Simulation: hostile loopback process, replayed iframe ticket, stale process generation, multiple panes, slow frame consumer, human/agent race, redirect/popup/external protocol, engine death between shell and first frame, retry after pane close.

Findings: origin/source/nonce alone were insufficient. Runtime instance, engine generation, pane, one-time ticket, control lease and frame ownership must be generation-bound. Navigation policy must be enforced at browserd for every top-level path. Queues require latest-frame-wins and exact-once CDP ack. Retry requires single-flight, total bounds and cancellation.

## Round 3 — packaging, update and rollback

Simulation: clean-room build, inside-out signing, notarization/stapling, Windows signing, update during an active session, failed new runtime, rollback, remote download verification.

Findings: runtime target/hash/protocol/license data require one manifest; signed app contents cannot be replaced in place; rollback selects only sealed compatible runtime units or disables Browser safely; release publication must atomically promote one exact asset/checksum set. Existing external Bun/Chrome fallback is development-only and cannot be a production rollback.

## Twelve code contracts

1. `cysd`'s lazy `BrowserAuthorityExtension` is the sole public lifecycle owner. Its private supervisor owns the process group and `EngineKey` lock. Tauri, `cys`, and Python never spawn the runtime.
2. `RuntimeManifest`, `RuntimeStateV2`, `EmbedDescriptor` and protocol envelopes are typed, bounded and tolerant only where explicitly versioned. Unknown security-sensitive fields fail closed.
3. Runtime control identity and embed access are separated. The UI receives no control-token DTO field; an embed is bound to runtime instance, engine generation, context, pane, ticket, parent origin and expiry/one-time engine consumption.
4. Pane state transitions are explicit. `CLOSED` is terminal for that generation; reconnect creates a new positive generation in `PENDING`. Spawn and iframe load are not readiness.
5. Presence, human-control and render/input ownership are separate leases. Release requires the current lease id, pane, client session and generation.
6. Resource order is fixed: WS zero → reconnect grace → screencast/CDP release → engine idle deadline. Browser cleanup never terminates cysd, PTYs or organization nodes.
7. Each client has at most one outstanding render; the queue is bounded/latest-frame-wins. Every rendered, dropped, evicted, detached or timed-out frame produces at most one CDP ack.
8. HTTP/HTTPS policy is enforced in browserd for initial navigation, address entry, redirects, history/reload outcomes, popup adoption, downloads and external protocols.
9. Start/retry is authority-gated, single-flight, bounded by attempts and elapsed time, and cancelled when the pane closes. Passive layout restore never starts Browser.
10. Runtime update follows stage → signature/hash verification → atomic selection → health → commit, with a durable rollback journal. Signed bundles are never mutated in place.
11. Release promotion covers two notarized/stapled macOS DMGs, Windows installer and ZIP, complete `SHA256SUMS`, updater manifest and download-site snapshot as one exact set. Windows Defender guidance and its remote grep are mandatory.
12. Gate A is fail-closed: existing PATH/PTY/CLI installation/Claude discovery/adapter ownership/bootstrap tests must be green before Browser release. A red baseline is repaired or explicitly separated; it is never waived by documentation alone.

## Dependency and ripple map

| Changed contract | Direct owner | Required consumers/evidence |
|---|---|---|
| lifecycle/authority | `src/bin/cysd/authority_broker`, private supervisor | Tauri commands, `cys`, Python adapter, boot-isolation tests |
| runtime identity/path | runtime manifest/state/path modules | supervisor, packaged engine env, release attestation/policy |
| embed descriptor | broker private endpoint reader | Tauri adapter, UI pane loader, cast ticket registry, persistence sanitizer |
| cast protocol/frame flow | browserd cast/server | UI state machine, WS integration, slow client/rebind tests |
| human control | browserd context arbiter | RPC mutation gate, pane owner, agent snapshot concurrency |
| navigation | browserd single policy | address bar, redirects, popup, download, external-protocol tests |
| persistence | UI web-pane state | layout migration, pending reconstruction, no token/port persistence |
| packaging/rollback | release workflows and scripts | macOS/Windows signing, checksums, remote site, recovery drill |
| freeze invariants | PATH/PTY/onboarding/pack code (unchanged) | pre/post snapshots and clean-HOME E2E |

## Bootstrap and orchestration invariants

The Browser extension is optional and lazy. No Browser code may enter cysd construction, PTY creation, ordinary-pane creation, PATH synthesis, pack reinject, or organization bootstrap. Browser failure is observable but confined to Browser. CEO/master plans, decides, delegates and supervises; workers implement/package; reviewer 1 performs adversarial verification; reviewer 2 audits dependency, documentation, packaging and invariants. Producer self-review is not release acceptance.
