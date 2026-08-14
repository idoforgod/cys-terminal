# cys-terminal

**An orchestration terminal for commanding fleets of AI agents.** Cross-platform: macOS & Windows.

A terminal multiplexer, a local daemon, a mission-control dashboard, and a multi-agent
operating system (the CYSJavis pack) in one body. Run several CLI agents (Claude Code,
Codex, …) in parallel under distinct roles — **master, worker, CSO, reviewer** — let them
talk to each other over sockets, and monitor cost, context, and hardware in real time.

> Most of this codebase was **written by AI agents under human direction** — the
> `Co-Authored-By` chain in the commit log is the record of that process. The
> repository itself is a working proof that AI-fleet orchestration is real.

*한국어 문서(전체 레퍼런스 포함)는 [README.md](README.md)를 보세요.*

## Docs

| Doc | Contents |
|---|---|
| **[Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md)** | Design theses, system architecture, security model, invariants (Korean) |
| **[User Manual](USER-MANUAL.md)** | Install to fleet operations, full CLI/env/protocol reference (Korean) |
| [INSTALL.md](docs/INSTALL.md) · [INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md) | Install details |
| [SECURITY.md](SECURITY.md) · [CONTRIBUTING.md](CONTRIBUTING.md) · [NOTICE.md](NOTICE.md) | Security reporting · contributing · third-party attribution |

## Why

Existing terminals and multiplexers are built for humans typing commands. Run several
AI agents in them and you hit real limits fast: panes cannot talk to each other, orphan
servers left behind by agents pile up until the machine chokes, and nobody can see who
is spending what. cys-terminal is an independent, from-scratch implementation that makes
those three problems first-class features.

And a **fourth problem** — *how do you organize the agents into an organization?* — is
solved by a built-in pack (**CYSJavis**: role-based absolute directives + deterministic
operational tools).

The resource wall is handled not only while agents run but **during deploy and upgrade**
too: keeping the "stop" command reachable even at the instant the app is replaced with a
new version (cross-platform atomic swap) is a first-class goal.

One documentation convention: when this repo describes an improvement, it **states what
the user loses first**. Marketing exaggeration is banned — only what actual code and
releases back up gets written down.

## Design Principles (ABSOLUTE)

1. **Bidirectional socket communication** — no one-way send + capture polling.
   Every pane on the same socket is an **equal node** that can actively push to any
   other pane by surface ID: `cys send --surface surface:31 "..."` + `send-key Return`
   injects **directly into the target pane's PTY stdin**, arriving as a new user turn.
   Server→client is the `cys events` push stream (sequence numbers, resume on reconnect).
2. **Resource governance as a first-class feature** — built-in mitigation that stops
   orphan-server accumulation → load explosion → 401/hang at the source.
3. **Core/UI separation** — the daemon (`cysd`) runs independently of any UI. Even if
   the UI hangs, the socket control channel stays alive (out-of-band recovery).
4. **Fail-closed signing** — app binaries are signed for the Tauri updater; the pack is
   signed with minisign (public key pinned in the binary). If verification fails,
   installation/deployment itself is refused. *Self-sealing invariant* — a signed bundle
   never rewrites its own contents at runtime (3-layer `.pyc` self-generation sealing +
   a post-signing count-reconciliation gate).
5. **Directives and machine as one body** — the role-based absolute directives,
   operational tools, and skills (the CYSJavis pack) are built, signed, and shipped
   together with the terminal, and auto-injected when a node boots.
6. **Passive cognition layer (radio)** — discovery notifications and decisions are kept
   physically separate. Principle 1 (active push) is the decision/steering channel;
   radio is the *passive* layer where many workers running tickets in parallel announce
   their **findings** to one another. Decision traffic — approvals, verdicts, done — is
   never carried on radio: the single source of truth for decisions stays the tickets
   and `gate-status`.

## Install

Grab the latest from [Releases](https://github.com/idoforgod/cys-terminal/releases/latest).
Recipients **do not install a daemon separately** — the app boots it and installs the
pack automatically.

- **macOS**: `cys_<version>_aarch64.dmg` (Apple Silicon). A bundled **"Install cys.app"
  helper** stages the app hidden, then swaps it into place with a single system call
  (`renamex_np`), eliminating the race where a Finder drag exposed a half-copied bundle
  mid-copy. (Use the helper rather than overwriting by drag.)
- **Windows**: `cys_<version>_x64-setup.exe` — daemon, CLI, and runtime bundled
  (self-contained). PE version-resource, manifest, and icon embedding reduce
  SmartScreen/Defender friction, but the build **is still unsigned, so a first-run
  warning can appear** (honest disclosure). See
  [docs/INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md).
- Optional 24/365 always-on: `cys daemon install` (launchd KeepAlive / Task Scheduler).
- Use `cys` from an external terminal: app Control Center → **"Install cys to shell"** (one click).

Install/uninstall details: [docs/INSTALL.md](docs/INSTALL.md). Full usage:
[User Manual](USER-MANUAL.md).

## Quick Start

```bash
cys identify                                  # who am I (surface address)
cys launch-agent --role worker --agent claude # boot a role node (directives auto-injected)
cys send --to worker "status report, please"  # push by role address
cys send-key --to worker Return               # confirm submission
cys status --json                             # one-call fleet snapshot
cys events --reconnect                        # push event stream (replaces polling)
cys run --scoped -- python -m http.server     # lifecycle-managed scoped execution
cys boot                                      # boot the standard node set (auto-detects installed CLIs)
```

## Architecture

```
cys.app  Tauri desktop app: terminal UI (xterm.js — wheel scroll, drag-select, copy
         restored even over a TUI) + Control Center — a thin client of the daemon
cysd     headless core daemon: NDJSON socket server (UDS / Windows named pipe),
         PTY (portable-pty: openpty / ConPTY), vt100 screen reconstruction, event bus,
         watchdog, process ledger, usage/cost collectors, persistent analytics (SQLite),
         scheduler
cys      CLI: the equal-node client used by the AI inside each pane
pack     cysjavis-pack/: 10 absolute directives · 90+ deterministic tools · 25+ hooks ·
         114+ skills · 4 schemas (embedded at build, minisign-signed at distribution,
         user-modified files treated as inviolable)
```

Every pane process gets `CYS_SURFACE_ID`, `CYS_SURFACE_REF`, and `CYS_SOCKET` injected
automatically — the AI inside a pane learns its own address instantly via `cys identify`.
The PTY is owned by the daemon, so sessions survive app restart, reinstall, and update
(re-attach).

## CYSJavis Pack — the built-in multi-agent OS

Install the terminal and connect an AI CLI, and a **master–worker–CSO–reviewer
multi-agent operating system** comes online. The system has three layers:

| Layer | Contents | Source |
|---|---|---|
| Core (machine functions) | Bidirectional sockets · approval Feed · watchdog/ledger · event push · session persistence | cys-terminal core |
| CYSJavis pack | Role-based absolute directives · deterministic operational tools · hooks · skills | `cys init-pack` |
| Personal layer | `soul.md` (priorities/red-lines) · long-term memory | **accumulated by you as you use it** |

The four roles are distinct: **master** decides and supervises, **worker** implements,
the **CSO** is the first responder for system/resource matters, and two heterogeneous
**reviewers** verify and rebut (they are not workers — they are the master's dedicated
verification/adversary reviewers).

**Master-declaration hierarchy fallback** — the owner's single sentence "you are the
master" auto-boots a 5-node team (boot and task-start are separated). A **second
declaration is not a conflict (rejected) but auto-creates a new department**, and on the
first department the existing master is auto-promoted to CEO. The department-creation
path is thus extended beyond the GUI button to a "declaration path." A declaration
*delivered or executed by an agent*, however, is stopped by a machine-origin gate and
does not trigger this fallback (human channel only).

`soul.md` and `memory/` ship as **intentionally empty skeletons** — the design belief is
that operating taste and long-term memory are not borrowed but filled in by the user.
Autonomous piloting (running an approved roadmap to completion unattended) turns on only
when the owner grants it explicitly in `soul.md`, and a **kill-switch that instantly
pauses on any owner input** has top priority. Symmetrically, the **authority to start**
an autonomous run comes **only from the owner channel** — even with unfinished work in
the queue, if no mission is assigned, a **mission gate** reports and halts, guarding the
start side opposite the kill-switch.

Details: [Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md) §2–4; operations:
[User Manual](USER-MANUAL.md) §12.

## What you get — three configurations compared

cys-terminal is different from a traditional terminal **even if you just connect plain
`claude`** with no Jarvis onboarding. Three configurations were compared across
33 items × 6 areas (fresh-machine E2E measurement skeleton + v0.14.x release-code
tracing + pack-number re-measurement):

- **①** Traditional terminal (iTerm, …) + claude CLI
- **②** cys-terminal + plain claude (everyday use, no Jarvis onboarding)
- **③** cys-terminal + Jarvis onboarding ("you are the master" → 5-node full system, **the baseline**)

The **①→②** gap is what you get **from installation alone** — auto-deployment of the
full skill set (a 570+ file pack) into an isolated config, observation, inter-pane
communication, `~/.claude` protection, and skills, categories that simply do not exist in
a traditional terminal. The **②→③** gap is organization, memory, and autonomy (Jarvis's
unique value): team formation, ticket/reviewer/RSI/eval loops, cross-session recovery
(`SESSION_STATE`/`RECOVERY`), radio finding-sharing, and autonomous piloting with a
denylist boundary. The only real friction in ② is two one-time gates:

> **F1** = Claude Code's own first-run dialog (5 steps: Enter → security note → terminal
> setup → folder trust → bypass warning) needs one human pass. **F2** = the isolated
> config keeps Keychain credentials per config path
> (`Claude Code-credentials-<sha256(path)[:8]>`), so it needs its own `/login` once.

Moving from ② to ③ is a **single sentence**: "you are the master."

## Head-to-head — Coral AgentRadio (full-repo audit, 2026-08)

The same concept — a passive-awareness broadcast layer where agents "hear while they work" — was published by academia
as an experiment and shipped by cys as a product. We audited [Coral-Protocol/AgentRadio](https://github.com/Coral-Protocol/AgentRadio)
(the reproduction repo for [arXiv:2607.28430](https://arxiv.org/abs/2607.28430)) **file by file, with measurements**, followed by
two independent adversarial reviews. As of 2026-08-14 (cys v0.14.15 ↔ AgentRadio HEAD 5e4e137).

> **Honest framing** — one side is paper-experiment code (3,318 executable lines), the other a product in daily operation
> for two months (151,307 executable lines). Different weight classes; the tables below report measured differences per axis,
> not a victory lap.

### Where Jarvis leads — 6 of 8 judged axes (all measured)

| Axis | cys / Jarvis | AgentRadio |
|---|---|---|
| Architecture (durability · recovery) | radio state is **files** (survives session clear & context compaction) · ack-cursor recovery · idempotency keys | server **JVM memory** (crash loses all conversation) · no dedup |
| Feature surface | 66 CLI subcommands · 68 RPC methods · 86 deterministic tools · 114 skills | 3 comm primitives + state read |
| Reliability · verification | **1,710 automated test cases** · 5 CI pipelines · radio: 297 assertions all PASS | **0** own tests · **0** CI · grader triplicated with divergent copies |
| Security | fail-closed dual signing · kernel-derived sender identity · ACL · pre-publish secret scan | static credential `test` · unchecksummed 106MB JAR · all agents in permission-bypass mode |
| Operations · observability | 9-tab Control Center · process ledger · watchdog · pre-flight resource gate | metering function is a no-op (token usage never collected) |
| Maturity · distribution | 149 releases in 2 months · notarized macOS DMG · zero-downtime pack updates | 1 release (v1.0.0) |

In the broadcast-layer head-to-head (17 items), **radio (Jarvis) leads on all 11 durability/security/verification items** —
machine-verified evidence with automatic claim demotion, two-stage secret scrubbing, and 4-level urgency with cooldowns have
no counterpart in AgentRadio.

### Where AgentRadio leads — 2 axes + 3 items (acknowledged as-is)

| Their strength | Detail |
|---|---|
| Published benchmark score | SWE-Atlas QnA, 124 tasks: 32.3% solo → 62.1% with 4 agents (arXiv + press). **cys has no public benchmark score yet** |
| External visibility · ecosystem | 3-paper arXiv lineage · VentureBeat coverage · README in 6 languages · MCP ecosystem (coral-server) |
| Zero-modification portability | copy a few shell scripts into any harness — cys radio requires the cys stack |
| Instant mention wake-up | server push returns immediately — radio's normal path polls at 5s (only BLOCKER gets direct stdin delivery) |
| Cross-model evidence | reports the same effect direction on both Opus and DeepSeek |

That 62.1% cannot be quoted at face value, though — five discount findings confirmed by the audit: no aggregation code in the
repo (unreproducible), an arithmetic error in the results table propagated to all 6 language READMEs, two lenient-bias defects
in the grader, not listed on the official leaderboard (self-reported), and statistical indistinguishability from neighboring scores.

### Our gaps — same yardstick, disclosed

No Windows Authenticode signing (no certificate) · no public benchmark score · external-adoption metrics unmeasured ·
one stale figure set in the architecture doc. Next step: obtain a benchmark score on the same public harness (Harbor)
under a producer≠evaluator protocol.

## Control Center (real-time monitoring + persistent analytics)

A dedicated full panel in the app — `cysd` serves fleet, usage, and system over a single
RPC (no external dashboard needed); persistent analytics accumulate in cysd's embedded
SQLite (`analytics.db`, graceful degrade if it can't open). Philosophy: **local-first**
(data never leaves the machine) · zero extra infrastructure · 0 ms agent latency (hooks
are fire-and-forget).

Nine tabs: **Live** (node fleet · per-core CPU / GPU / NPU / memory at 2 s · today's
tokens/cost/model-mix · alert strip) · **Cost & Efficiency** (persistent aggregates,
token 4-way split, per-model cost with unknown-price flagged, cache savings/reuse) ·
**Skills & Agents** (skill/tool/delegation call counts, failure rate, repeat failures) ·
**Sessions** (timeline, activity ribbon, transcript-excerpt drill-down, favorites, PII
redaction) · **Trends & Weekly** (WoW deltas, efficiency leaders, skill assets) ·
**Learning** (RSI round timeline, adopt/rollback, cumulative discoveries) · **Skill
Board** (a curated skill button = a one-shot worker run with HITL preview) · **Work**
(current tasks across every department × node, observe-only, self-report vs. derived
trust badges) · **Approval Feed** (Allow/Deny). Plus: ⌘K Command Palette, Glance mode
(⌘G, a summary screen for non-technical users), workspace groups, **departments**
(project isolation via independent daemons), and RBAC PII redaction
(`CYS_CONTROL_REDACT=1`).

## Jarvis-native features (22)

> Design thesis: **every operational duty the directives ask the orchestrator to perform
> by hand = a feature gap in the terminal.** ① convention → daemon-guaranteed
> mechanization ② self-report first, screen-parsing is the fallback ③ 3-tier automation
> safety (alert → escalate → act, deny-by-default).

- **T1 — identity & reporting**: self-reported status/context%/task (`cys set-status`);
  one-call fleet board (`cys status`, `cys fleet`); sender identity + role→role ACL
  (kernel peer-pid `from` verification).
- **T2 — resilience**: context-cycle executor (save → file gate → clear → re-inject →
  resume, `cys cycle-agent`); instant agent-death detection + optional recovery (blocked
  on auth error); org restore (topology persistence + bulk re-boot/re-inject); directive
  drift detection/re-injection (`cys reinject`); orchestrator dead-man (`master.deadman`).
- **T3 — coordination**: todo watch (per-role TODO mtime → progress rollup); one-shot
  timers with fresh TTL; role-glob broadcast (`cys send --to 'reviewer-*'`); feed-aging
  re-alert; input safety (typing guard, atomic authority delivery); delta read + wait
  (`cys read-screen --since N`, `cys watch --until <re>`).
- **T4 — safety & attestation**: **kill-switch** (`cys pause/resume`, `cys gate-check`);
  approval escalation (screen scan → event + feed, never auto-answers); health-rule
  action binding (opt-in, pauses queued delivery only); transcript hash-chain
  attestation (`cys attest pin/verify`, producer≠evaluator); recall retention policy.
- **T5 — cognition & attribution**: **radio** — parallel workers share findings, FACT
  truth-checking (file·line·snippet, unverified is auto-demoted), BLOCKER gate, decision
  traffic forbidden (`javis_radio open/send/wait/read`); **BOOT_SNAPSHOT** — restores
  memory as a read-only ledger digest after clear/compact (`javis_snapshot generate`);
  **attribution-arbitration ledger** — on suspected pane-text forgery, the delivery
  ledger is consulted before any fix begins; no-evidence attribution is void
  (`javis_mission delivery-path`, machine-origin).

## Resource governance (three mitigations)

| Mitigation | What it does | Command / event |
|---|---|---|
| ① Login-loss detection | Every output line is matched against health rules (default: `Not logged in` · 401 · token expired · rate limit) → 30 s debounced push. **Self-amplification is blocked**: alert lines mask their own triggers (send-side sealing) and lines *discussing* an alert are excluded from matching (receive-side isolation). | `health.alert` · `cys add-health-rule <name> <regex>` |
| ② Short work units | Idle detection (default 300 s of no output) pushes → split/check decision. | `pane.idle` event |
| ③ Forced server teardown | **Scoped execution** (new process group + ledger, torn down as a group on exit) · **close-surface** (child tree killed) · **watchdog** (load / child-count / duplicate-command detection). | `cys run -- <cmd>` · `cys ps` · `cys kill <pid>` · `watchdog.*` |

## Approval Feed · in-flight queue

```bash
cys feed push --wait --title "approve git push" --body "..."   # blocks until decided (exit 0=allow, 2=deny, 3=timeout)
cys feed reply <request_id> allow                              # CLI, or the UI Allow/Deny buttons
```

There is **no auto-answer** (human-in-the-loop) — the daemon even refuses a requesting
node's self-approval. Repeated-risk commands are passed by signing once with
`cys approval sign` (master-only, HMAC signed-prefix).

- Default send (`cys send`) = **steer**: injected into stdin immediately, absorbed as
  steering while the target is running.
- `cys send --queued` = **followup**: delivered one item per beat once the target has
  been quiet for 3+ seconds.

## Updates — dual channel + zero-downtime

| Badge | Channel | How |
|---|---|---|
| `!` | App (binary) | Tauri updater signature verify → session guard → install/restart → pack applied + nodes auto-return |
| `↻` | Pack (OS) | **Zero-downtime** — minisign verify → atomic transaction → live-node re-injection. No restart; sessions and daemon survive |

Checked quietly at startup and every 6 hours. If a "disk has the new version, process is
the old daemon" skew remains after reinstall, it resolves via a badge-click handover or
idle auto-handover (when there are 0 live sessions — lossless). Diagnose/repair with
`cys doctor [--fix]`; self-diagnose the installed build's code-signing seal with
`cys doctor app-seal`.

**Coexists with customization**: updates never destroy user-modified files — user-owned
files are preserved with the new version placed alongside as `.new`, system files are
preserved as `.user` before healing, and the `~/.cys/local/` overlay (directive append,
skill shadowing, hook trailing) is invisible to updates. Preview with `cys pack-plan`;
merge with `cys pack-merge` (3-way / AI).

## Channel bridge (Slack · Discord)

Export the fleet's approval requests and reports to an external messenger and accept
remote approvals from permitted senders — sender allowlist · separate grant for remote
approval · instant lockdown · shape-based redaction built in. See `cys channel status`.

## Protocol · environment variables

NDJSON (one line = one JSON), dozens of RPC methods + 13 `channel.*` methods, dozens of
events. The exhaustive lists and the environment-variable table are in
[User Manual §16–17](USER-MANUAL.md).

## Source build (for contributors)

```bash
git clone https://github.com/idoforgod/cys-terminal
cargo build --release
./target/release/cysd &                       # daemon (duplicate boot auto-refused)

cd ui && sh build.sh                           # frontend bundle (bun)
cargo build -p cys-app                         # dev run: ./target/debug/cys-app
bun x @tauri-apps/cli build                    # distribution bundle
```

Note: rebuild the app after editing `ui/` (the frontend is embedded in the binary). The
PTY is daemon-owned — sessions persist across UI restart and app reinstall (re-attach).

## Security model

- No network listener — user-owned Unix socket (macOS) / DACL-sealed named pipe (Windows) only.
- Sender identity verified by kernel peer pid (self-report distrusted) · role→role ACL ·
  capability gates are deny-by-default (reviewers are read-only).
- **Attribution (who sent it) is decided by the delivery ledger, not screen strings** —
  self-report and pane text are distrusted, and an attribution claim without ledger
  evidence is treated as void.
- Dual-signed updates — app via Tauri updater, pack via minisign (public-key binary pin ·
  replay monotonicity · fail-closed).
- No approval auto-answer (HITL) · self-approval blocked · external URLs are a hard
  allowlist (extendable only via local config).
- Pre-publish secret/PII gate: `scripts/secret-scan.sh --all` (fail-closed). Invisible-
  character bypasses are blocked by **Unicode-category coverage** (removing Cc/Cf/Zl/Zp),
  not a hardcoded enumeration ("enumeration loses to the next bypass — block by category").
- **Release gate**: before a release, CI reproduces the real user path — on macOS it
  attaches quarantine to a copy and runs Gatekeeper for real (spctl/codesign/stapler),
  on Windows it verifies by PE reputation; if it can't pass, the upload itself is blocked
  (fail-closed · reproducing the path the user actually walks, not a static pattern).

Report vulnerabilities per [SECURITY.md](SECURITY.md); details in
[Architecture & Philosophy §6](ARCHITECTURE-AND-PHILOSOPHY.md).

## Known limitations (honest disclosure)

- On macOS, if sysinfo can't read the full cmdline, processes are grouped by name
  (possible over-grouping).
- If the CLI dies via Ctrl-C during `cys run`, group cleanup falls to the watchdog cycle (5 s).
- Real-time GPU/NPU in Control Center is currently macOS (Apple Silicon) only — Windows
  shows CPU/MEM only. NPU has no utilization-% public API, so actual power (W) is shown.
- Single-UID trust model — approval signing and self-approval blocking are a
  detection/fail-safe layer, not a cryptographic defense against a malicious process
  inside the same account.
- The **mission gate** cannot cryptographically stop same-UID forgery — it is handled as
  a delivery-ledger audit trail.
- The **Windows upgrade-atomicity** repair goes as far as code review and model
  verification on a Mac dev machine; real-hardware confirmation is in progress
  (honest disclosure).
- The **macOS build is unsigned** — the installer helper lowers friction, but a first-run
  warning can still appear, and "half-install vs. quarantine" is disambiguated with
  `cys doctor app-seal`.
- **radio** cannot in principle guarantee cross-channel exactly-once or a zero-miss
  window (unresolvable — a managed residual risk).

## Troubleshooting · reset

- macOS **"damaged and can't be opened"** has two causes — ① a half-install (drag-copy
  race) or ② the quarantine attribute. Disambiguate with `cys doctor app-seal`; prefer
  the **"Install cys.app" helper** to install.
- For a **full reset** (including Windows WebView2 stored values and leftover department
  isolates), follow [docs/GUIDE-clean-reset-KR.md](docs/GUIDE-clean-reset-KR.md).

## Contributing · License

See [CONTRIBUTING.md](CONTRIBUTING.md); third-party attributions in [NOTICE.md](NOTICE.md).
MIT License ([LICENSE](LICENSE)) · Contact: **cysinsight@gmail.com**
