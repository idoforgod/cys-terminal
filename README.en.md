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
  warning can appear**. See
  [docs/INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md).
- Optional 24/365 always-on: `cys daemon install` (launchd KeepAlive / Task Scheduler).
- Use `cys` from an external terminal: [docs/INSTALL.md](docs/INSTALL.md) §B (one-time manual symlink).

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
cys run -- python -m http.server              # lifecycle-managed scoped execution
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

## JavisRadio vs AgentRadio — where we lead, where we fall short

cys's passive-awareness layer (Design Principle 6 · T5-20) is a re-implementation of the
three primitives of **AgentRadio** (arXiv:2607.28430) by Coral Protocol, hardened with
machine gates. The original research showed that letting four coding agents listen
*while* they work lifts SWE-Atlas QnA task accuracy from 32.3% (single agent) to 62.1%
(four agents, McNemar p=0.0023), with a DeepSeek replication (29.0%→50.8%, p=0.0026) and
a **B1 budget control** (one agent with 6× budget still reaches only 37.9% — blocking
the "you just spent more compute" objection by design): textbook experimental work.
Below is the verdict of a full source-level survey of the original paper and repo
(2026-08-14; triple-verified — two independent sessions + two adversarial reviewers +
number re-execution), scored on 10 axes — **wins and losses first**.

### At a glance — from cys/Jarvis's side: 8 ahead · 1 conditional · 1 behind

| # | Axis | Verdict | One-line reason |
|---|---|:---:|---|
| 1 | Communication (passive awareness) | ⚠️ conditional lead | ports the 3 primitives + 14 defense commands · zero-loss surfacing (duplicates are audited exceptions) · retraction with contamination-cascade closure · idempotent queue — but **the concept and the field data belong to AgentRadio** |
| 2 | Role topology | ✅ ahead | heterogeneous three-vendor reviewers (claude · agy · codex) block correlated errors vs four same-model agents (a shared blind spot goes uncaught) |
| 3 | Verification & quality gates | ✅ ahead | completion claims without evidence are machine-rejected via exit codes + a four-party convergence gate vs a pipeline whose only machine gate is checking that answer.txt exists — unanimity is a prompt sentence ("count the APPROVEs yourself") |
| 4 | Recovery & durability | ✅ ahead | multi-layer recovery canon (SESSION_STATE · RECOVERY · persistent todos) + repairs born from real incidents (message-loss bug AA20 → single critical section; a 72%-quota burn → mission gate) vs single-layer process resume — server death = team state gone, token expiry = spin |
| 5 | Resource control | ✅ ahead | pre-start resource gate · process ledger · group cleanup vs none (relies on container disposal) |
| 6 | Human-in-the-loop | ✅ ahead | Approval Feed (exit 0/2/3) · kill-switch · denylist boundary vs "do NOT ask for human input" as the spec |
| 7 | Everyday generality | ✅ ahead | daily operation + 114 skills + departments + offline-local (zero network listeners) vs a single-domain benchmark reproduction requiring Docker + Modal cloud + pinned Harbor (17 days of repo activity) |
| 8 | Shipping maturity | ✅ ahead | notarization · dual-channel signed auto-update · 6 platform targets · release-gate CI vs no packaging · hardcoded version '0.1.0' · a checksum-less 106MB JAR from Google Drive |
| 9 | **Measured performance proof** | ❌ **behind** | AgentRadio proved its method on a public benchmark — 124 tasks × 4 configs × 2 model families with statistical testing — **we have no system-level accuracy measurement** (remediation started: JAVIS-BENCH, a pilot on the same task set) |
| 10 | Ecosystem | ✅ ahead | 86 deterministic tools + 114 skills + heterogeneous CLI adapters **already running in-house** vs 3 primitives (an MCP open-protocol agent-ecosystem ambition exists on their side) |

> Fairness note: AgentRadio is a **research artifact** built to prove one hypothesis, so
> the absence of axes 5·6·8 is outside its design goal. Read the per-axis evidence, not
> the totals; the axis-9 loss is our named next task. On judging (a same-vendor AI
> judge): the same judge is fixed across all configurations, so **the bias cancels out in
> the L2→L3 relative comparison** — what it does threaten is the absolute numbers and the
> "leaderboard #1" narrative (the current single-agent leader at 63.17% exceeds 62.1%,
> though the ~±5 confidence intervals overlap so neither direction is statistically
> settled; AgentRadio is self-reported, not on the leaderboard). Cost, per the authors'
> own figures: $2.96 → $19.45 per task (6.6×).

### The quantitative scale — size and depth, all re-measured

| Metric | AgentRadio | cys/Jarvis stack |
|---|---|---|
| Code size | ~3,300 lines (Python 2,017 + shell 1,301) | **~169,000 lines** (Rust 63,371 + pack Python 105,833 + more) = **~50 : 1** |
| Self tests | **0** (no tests or CI for its own harness code) | **~1,700** — Rust `#[test]` 883 (src) · 920 (whole repo) + pack 531 + radio 297 (incl. 23 red-team cases; re-run same-day, all PASS) + 16 UI test files |
| Communication surface | 3 primitives | 66 CLI subcommands (incl. 17 radio subcommands · a 10-code exit contract) |
| CI | none (1 visible commit) | 5 lanes + flaky-test gate + notarization regression check |
| Benchmark assets | **124 tasks · 1,306 rubrics · contamination canaries · statistical testing** (their strongest suit) | none — JAVIS-BENCH started to close this |

> The 50:1 ratio cuts both ways — evidence of our depth, and of our complexity; their
> smallness (fully auditable in an afternoon) is a scientific virtue, though one
> undercut by an unauditable 106MB server binary.

### ❌ Where we fall short — all of it, from the same survey

| # | Gap | Fact |
|---|---|---|
| 1 | **Zero public benchmark evidence** | we have no outcome-level proof that our orchestration raises task scores — JAVIS-BENCH (single agent vs Jarvis-style orchestration on the same SWE-Atlas QnA tasks) has been started to close this |
| 2 | **Intellectual priority is theirs** | passive awareness and the 3 primitives are AgentRadio's; our own spec declares the port. Ours is a hardened port |
| 3 | **Single machine, no node auth** | radio is single-machine and unauthenticated (the name 'master' is always trusted) — their MCP server aims at cross-framework, multi-host reach |
| 4 | Other known gaps | no published cost figures (they publish theirs, down to the 6.6×) · Windows binaries not Authenticode-signed · radio's own docs admit full exactly-once and zero deaf-windows are not guaranteed (the hardening is partial) |

### Detail — the radio layer 1:1 (the evidence behind the structural lead)

| | AgentRadio (original research) | JavisRadio (cys pack) |
|---|---|---|
| Surface | 3 primitives (create_thread / send_message / wait_for_mention) | those 3 + 14 defense commands = 17 subcommands |
| Broadcast truth | none — content relayed as-is | FACT claims machine-verified against evidence (file · line · snippet); failures auto-demoted to hypothesis/unverified |
| Duplication / loss | no idempotency keys, acks, or sequence numbers (mention delivery itself is a server push; the timeout-fallback detection is grep string-counting) — dedup delegated to LLM cognition | monotonic seq + separate surfacing/acceptance cursors — invariant hierarchy "never zero > never twice" |
| Retracting a false broadcast | no concept | `retract` — closes the contamination cascade, including broadcasts that cited it |
| Completion gate | the only machine check is that answer.txt exists (2-hour polling) — unanimity is a prompt sentence | done-check rejects unsurfaced/unresolved broadcasts via exit codes (10-code contract) |
| Infrastructure | resident 106MB message server (auth key 'test') — server death = team state gone | no resident server — append-only files are the source of truth (rotation keeps seq continuity, archive after close) |
| Abuse defense | none | cooldowns · per-sender circuit breaker · secret masking · record cap · pause isolation |
| Verifiers | four same-model agents agreeing with each other | heterogeneous three-vendor reviewers combined with producer≠evaluator gates |

JavisRadio's quality evidence is 297 checks across 73 sealed cases — including 23
red-team regressions, adversarial tests locking "a violation must be stopped by the
exact exit code". The axis-9 gap is being closed through the JAVIS-BENCH main
experiment.

## Jarvis stack vs Hermes Agent — head-to-head with a heavyweight platform

We compared NousResearch **Hermes Agent** (230,268 stars · 1,377,316 lines of executable code —
measured 2026-08-14 / GitHub API) via a full-repo survey plus independent adversarial reviews. It is
the bigger system (~9× code, ~20× test cases). Nine-axis verdict: **Jarvis ahead on 1 · Hermes ahead
on 4 · split on 3**; a same-layer comparison of 14 capabilities: **Jarvis ahead on 6 · Hermes ahead
on 5 · even on 3**. The six where Jarvis is ahead are all about **making results trustworthy** (trust
& control); the five where Hermes is ahead are all about **running anywhere, cheaply and
conveniently**. In one line: **Hermes broadened capability; Jarvis locked down procedure.** Neither
side has public benchmark scores (our JAVIS-BENCH is underway). Full evidence: the lane report
`JAVIS-vs-Hermes-Agent-종합성능비교보고서-2026-08-14.md` (adversarial-review-hardened).

### ✅ Where Jarvis is ahead — trust & control (6)

| # | Capability | In plain words (all measured) |
|---|---|---|
| 1 | A broadcast channel between agents | Jarvis has a radio (JavisRadio): news from teammates flows in while you keep working. Hermes core has no such broadcast (verified exhaustively — its closest thing is a board that agents poll) |
| 2 | Workers reporting on their own | Hermes child agents have their send capability stripped; the boss must open their logs. Jarvis workers push reports straight to the master |
| 3 | Machine-checked "done" | Jarvis rejects an evidence-free "done" by machine. Hermes has checking tools too, but the AI must choose to use them — and its always-on guard describes itself as an advisory that "never blocks completion" |
| 4 | A review bench from different vendors | Jarvis seats Claude, Gemini and Codex reviewers full-time, wired into pass/fail gates (so they don't share the same blind spots). Hermes has multi-vendor *advice* and delegate-to-other-AI skills — but not a verdict gate |
| 5 | Agents organizing their own team | The Jarvis master issues tickets, launches workers and checks convergence entirely through tools. Hermes's team topology (verifier/synthesizer) can only be set up by a human at the command line |
| 6 | Braking before work + signed releases | Jarvis blocks work before it starts if resources fail the check, and refuses to install a release whose signature fails. Hermes has no signing/notarization in CI, and its scripts quietly skip signing when credentials are absent |

### ❌ Where Hermes is ahead — all of it

| # | Capability | Fact | Why the gap exists (report §7-1) |
|---|---|---|---|
| 1 | Size & breadth | 9.1× code · ~20× tests · 87 tools · 197 skills · 22 messengers · 7 execution backends | Half choice, half homework — Jarvis built messaging too, but narrowly: two approval/report-only bridges (see 'Channel bridge' below). We kept "data never leaves the machine" instead of broadening; the width itself is genuinely behind |
| 2 | Command safety checks | A 4,919-line dangerous-command engine (111 patterns · de-obfuscation · an AI judge) | Half choice, half homework — Jarvis puts its checks before work starts and after results come out; the thinner mid-run check is homework we intend to import |
| 3 | Cost tracking & worker policy | Auto-sums what child agents spend / risky child commands denied by default | Cost rollup is homework for Jarvis. The approval-policy difference is half choice, half homework |
| 4 | Conversations that follow you · open standards | The same conversation continues across terminal↔app↔messengers / connects to outside AIs via three standards (A2A · MCP · ACP) | For a conversation to follow you across devices, data must leave the machine — a direct conflict with our principle (choice). Standards are half choice, half homework |
| 5 | Age & adoption | 230,268 vs 27 stars · 398 contributors vs effectively one person · 388 vs 62 days | Neither philosophy nor homework — a function of calendar and headcount |

### Where the differences come from — one paragraph

The two systems aim at different things. Hermes aims at "**a capable assistant that goes everywhere
with you**" and broadened its surfaces (their design doc: "capability lives at the edges"). Jarvis
aims at "**on my machine, with results I can trust**" and locked its boundary (our design doc:
"local-first — data never leaves the machine"). Taking the 20 axes/items where Hermes is ahead one
by one: **8 are homework Jarvis must do** (public benchmark, Windows signing, supply-chain checks,
…), **11 are differences born of the different aim** (wholly or partly), **1 is a function of
time**. We applied the same yardstick to Hermes's own weak spots. Item-by-item evidence (file:line)
and the full table: report §7-1.

## Jarvis stack vs OpenClaw — the layer the world's #1 wrote down as "will not build"

**OpenClaw** (386,229 stars · 81,179 forks · 372 registered contributors — measured 2026-08-14 /
GitHub API) is the most-starred software project in GitHub history. It is a **personal AI assistant**:
message it on WhatsApp, Telegram or 24 other channels and it works on your machine. We cloned the whole
repository (commit db4379bd) and compared it across 10 areas — survey, scoring and rebuttal were done
by different agents, and the rebuttal pass returned **0 false claims · 10 corrections**, all applied.
Verdict: **Jarvis ahead on 4 · even on 1 · OpenClaw ahead on 5**, and they are the bigger system
(~86× production code, ~14,000× stars).

In one line — the two lines they put on their **"What We Will Not Merge"** list (VISION.md:134-135),
"agent-hierarchy frameworks" and "heavy orchestration layers", are exactly what Jarvis is. Full
evidence: the lane report `자비스-vs-OpenClaw-전수조사-최종보고서-2026-08-14.md`
(adversarial-review-hardened).

### ✅ Where Jarvis is ahead — command, verification, control (4)

| # | Capability | In plain words (all measured) |
|---|---|---|
| 1 | Running many AIs as an organization | Jarvis **checks by machine that four seats are alive** — chief of staff, worker, and two reviewers from different vendors — and can clone a whole department. OpenClaw agents spawn children at depth 1 by default, with no master/worker/reviewer roles |
| 2 | Filtering results before trusting them | Jarvis **machine-rejects an evidence-free "done"**, and reviewers use four closed outcomes (accept/revise/block/escalate) instead of scores, with a counter-argument required before passing. In OpenClaw, verifying a child's result is **one sentence of guidance**, and there is no verifier module |
| 3 | Security (narrow lead) | Jarvis opens no inbound door, persists risky-command approvals with unforgeable signatures, and has a kill switch plus a pre-flight resource check. OpenClaw's own security engineering is top tier, but in early 2026 it saw **135,000+ instances exposed without a password**, a one-click remote-execution flaw (patched next day), 341→824 malicious skills, and a Chinese government usage restriction |
| 4 | Release & supply-chain integrity | Jarvis notarizes automatically, signs app and pack separately, and **refuses to install if even one file is missing from the signed manifest**. OpenClaw's dependency hygiene is exemplary, but its open skill marketplace actually shipped malware |

**Even (1) — recovery**: they are stronger at protecting the conversation store; Jarvis is stronger at
restoring organizational state and at **blocking a convincing but false restore** (claims are
reconciled against measurement right after recovery).

### ❌ Where OpenClaw is ahead — all of it

| # | Capability | Fact | Why the gap exists |
|---|---|---|---|
| 1 | Messaging channels | 26 channels + duplicate suppression across the delivery path / Jarvis has 2 | Half choice, half time — the owner locked the scope ("exclude Telegram, Slack and Discord only", 2026-07-04) and two is what one person can maintain. The inconvenience is real: **if you only use Telegram, you cannot approve from your phone.** On duplicates, **23 of their 26 channels (88%) are in the same position as Jarvis** |
| 2 | Tests & CI | ~120,000 test calls · 86 CI lanes / Jarvis has 1,664 · 5 | Mostly a size difference — our production code is 1/86th, so per thousand lines it is 41 vs 27 (1.6×). But **having zero tooling to measure which code never runs** is homework with no excuse |
| 3 | Feature breadth | 152 extensions · iOS/Android apps · UI in 21 languages / Jarvis has 114+ skills · macOS and Windows | Mostly choice — **83 of their 149 manifests (56%) are per-vendor adapters**, so the two sides count different things. Having no marketplace is also a choice (our installer rejects any file absent from the signed manifest, which cannot coexist with an open one) — the cost is that **you cannot install someone else's add-on**. No Linux, however, is homework: the code is ready and only the release matrix has no slot |
| 4 | Users & community | 386k stars · hundreds of contributors · major press / Jarvis has 28 stars · 53 forks | Mostly direction and time — **publishing externally is blocked without owner approval**, so going public is the exception (the repo and releases are public; 20,624 asset downloads across the last 12 releases). But **not having proven "is this better than a single AI?"** is homework — we ran the same tasks and our pipeline lost 2–0 to a single agent (raw evidence was summarized away between stages) |
| 5 | Documentation | 770 documents · auto-translated into 20 languages / Jarvis has 254, Korean | Choice — the primary readers of those documents are **programs, not people** (directives are injected into each agent, and a failed injection stops it from starting). An English README exists. Having **no documentation site** is homework |

### Where the differences come from — one paragraph

The two systems aim in opposite directions. OpenClaw aims at "**an assistant anyone can use
anywhere**" and broadened its surfaces — which is why its own vision document says it **will not
build** agent hierarchies or heavy orchestration layers. Jarvis aims at "**running many AIs so the
results can be trusted**" and made exactly that layer its core. Taking the 12 items where OpenClaw is
ahead one by one: **4 are homework Jarvis must do** (coverage measurement, performance evidence,
Linux, a documentation site); the rest are differences born of the different aim, or of time and
headcount. We applied the same yardstick to them — in OpenClaw, extensions run with the same
privileges as the core, and damage from installing a bad extension is not even accepted as a security
report under their policy. That is the price of breadth and openness. And our own messaging layer is
itself **what we learned by surveying OpenClaw** (no code copied — rules re-implemented, with what we
rejected written down too).

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

## Known limitations

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
 .
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
