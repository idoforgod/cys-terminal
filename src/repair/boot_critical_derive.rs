//! Build-time derivation of the boot-critical list (T5a · C4 4.1–4.5, 4.10 · RULINGS B6, W1, Y3).
//!
//! This file is compiled TWICE and declared in no `mod.rs`:
//! - by `build.rs` (`#[path = "src/repair/boot_critical_derive.rs"] mod boot_critical_derive;`),
//!   which writes the compiled table to `OUT_DIR/boot_critical.rs`;
//! - by the library's tests (`#[cfg(test)] #[path] mod derive;` in `boot_critical.rs`), so the
//!   anchors, the closure rule and the census are unit-tested and the compiled table is compared
//!   with a fresh derivation of the current sources.
//!
//! Rules (B6): Rust std only (no build-dependency), no process is started and no Python is run —
//! every source is read as TEXT with CRLF folded to LF; a missing anchor, or a registrar call site
//! outside the anchors, is an `Err` whose message names it (build.rs turns it into a build failure).
//! No registrar is modified: the anchors are read, never written (P1).
//!
//! The rule (written out in `impl/plans/T5a.md` §2):
//! 1. `hook` = every script the product registers AUTOMATICALLY into a Claude settings file
//!    (Rust `AWAKENING_HOOKS`; preflight `SELFCORR_HOOKS`, `CAPGATE_HOOK`, `APPBUILD_HOOK`,
//!    `GRILL_HOOKS`, class `EVENT_HOOK`, C08 `_register_hook`), minus the statusline scripts
//!    (C32 `_register_statusline`, agy `SCRIPT`/`SCRIPT_CMD`) — O-C4-4.
//! 2. `hook-dep` = files a listed hook binds structurally (source line; a variable whose value ends
//!    with a pack path; preflight `HOOK_BODY_FILES`; column-0 Python imports), transitively.
//! 3. `start-script` = proof-anchored scripts the daemon / app / boot chain run at start or
//!    restore; `start-dep` = their closure by rule 2.
//! 4. `config` = proof-anchored core config (`agents.json`).
//! 5. `protection` = the sibling list of the four kept protections (D:941), separate rows.
//! 6. clear-procedure files (session state, todo files, statusline) are never entries (4.3, F2).
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

/// One derived row (strings because the build script cannot see the library's enums).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenEntry {
    pub rel: String,
    /// `hook` | `hook-dep` | `start-script` | `start-dep` | `config` | `protection`
    pub category: &'static str,
    /// `hook-replay` | `sh-syntax` | `py-compile` | `json-parse` | `self-test` | `none-prerehearsal`
    pub check: &'static str,
    pub check_arg: String,
    pub timeout_s: Option<u32>,
    pub lifeline: bool,
    /// Why the row exists (anchor / binder) — provenance, not part of the O-C4-3 entry format.
    pub why: String,
    /// What proves the row (RULINGS AL1): `hook-replay` (class 1), `bootstrap-replay` (class 2),
    /// `real-level` (class 3 — the stage-5 end-to-end derivation test); `` for protection rows.
    pub proof: &'static str,
}

/// The texts the derivation reads. `pack_file` takes a pack-relative rel (`bin/x.py`),
/// `repo_file` a repository-relative path (`src/pack.rs`, `cysjavis-pack/bin/x.py`).
pub struct Sources<'a> {
    pub pack_rels: &'a [String],
    pub pack_file: &'a dyn Fn(&str) -> Option<String>,
    pub repo_file: &'a dyn Fn(&str) -> Option<String>,
}

/// Repository paths of the text anchors (build.rs watches the library ones).
pub const PACK_RS: &str = "src/pack.rs";
pub const AGY_RS: &str = "src/agy_statusline.rs";
pub const LIFELINE_RS: &str = "src/repair/boot_critical.rs";
/// Pack rel of the preflight (registrars and hook constants).
pub const PREFLIGHT: &str = "bin/javis_preflight.py";

/// A start / config / protection row proven by a literal at a code site. The build fails, naming
/// the anchor, when the proof file is missing, the literal is absent or the pack file is absent.
pub struct ProofAnchor {
    pub proof: &'static str,
    pub literal: &'static str,
    pub rel: &'static str,
    pub category: &'static str,
    pub check: &'static str,
    pub check_arg: &'static str,
    pub why: &'static str,
    /// RULINGS AL1: `bootstrap-replay` (class 2 — measured at the bootstrap layer), `real-level`
    /// (class 3 — named by the design or start-critical through a stated real-machine
    /// dependency), `` for protection rows.
    pub level: &'static str,
}

pub const PROOF_ANCHORS: &[ProofAnchor] = &[
    // ── class 2 (RULINGS AL1): breaking it stops the start at the bootstrap layer (measured) ──
    ProofAnchor {
        proof: "cysjavis-pack/hooks/role-bootstrap-legacy.sh",
        literal: "BOOT=\"$PACK/bin/javis_bootstrap.py\"",
        rel: "bin/javis_bootstrap.py",
        category: "start-script",
        check: "py-compile",
        check_arg: "",
        why: "start: the role-bootstrap body hands the team boot to it (the app's boot button runs the same chain); broken → the boot fails (bootstrap replay)",
        level: "bootstrap-replay",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_bootstrap.py",
        literal: "orchestra = os.path.join(PACK, \"bin\", \"javis_orchestra.py\")",
        rel: "bin/javis_orchestra.py",
        category: "start-script",
        check: "py-compile",
        check_arg: "",
        why: "start: boot step 5 runs `orchestra check`; broken → EXIT_CHECK, no completion marker (bootstrap replay)",
        level: "bootstrap-replay",
    },
    // ── class 2 by break mode (ii) (RULINGS AN2): the start path obeys the file's ANSWER as a
    //    stop — the file giving that answer stops the boot at the bootstrap layer (measured) ──
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_bootstrap.py",
        literal: "gate = os.path.join(PACK, \"bin\", \"javis_resource_gate.py\")",
        rel: "bin/javis_resource_gate.py",
        category: "start-dep",
        check: "py-compile",
        check_arg: "",
        why: "start: with a team deficit the boot obeys its hard answer (exit 2) → EXIT_RESOURCE_HARD 9, no team (bootstrap 3176-3233 · 3893; formation 505-523 likewise); mode (ii) stops the boot (bootstrap replay)",
        level: "bootstrap-replay",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_bootstrap.py",
        literal: "    import javis_lock as _lock",
        rel: "bin/javis_lock.py",
        category: "start-dep",
        check: "py-compile",
        check_arg: "",
        why: "start: the boot's single-flight lock — its BUSY answer folds the run into the exit-11 loser, no team (bootstrap 1526-1548 · 1447); mode (ii) stops the boot (bootstrap replay)",
        level: "bootstrap-replay",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_bootstrap.py",
        literal: "    import javis_lane as _lane_mod",
        rel: "bin/javis_lane.py",
        category: "start-dep",
        check: "py-compile",
        check_arg: "",
        why: "start: the lane rules — a malformed-lane answer, or a department other than the pack's, is the exit-8 lane guard (bootstrap 3511-3539); mode (ii) stops the boot (bootstrap replay)",
        level: "bootstrap-replay",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_bootstrap.py",
        literal: "    import javis_budget as _budget_mod",
        rel: "bin/javis_budget.py",
        category: "start-dep",
        check: "py-compile",
        check_arg: "",
        why: "start: the time limits of every boot wait — a zero limit is obeyed (ping 0 s → EXIT_PING; bootstrap 235-267 · 3621-3646; orchestra check 361-383); mode (ii) stops the boot (bootstrap replay)",
        level: "bootstrap-replay",
    },
    // ── class 3 (RULINGS AL1): named by the design / start-critical on a real machine ──
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_bootstrap.py",
        literal: "os.path.join(PACK, \"bin\", \"javis_preflight.py\")",
        rel: "bin/javis_preflight.py",
        category: "start-script",
        check: "py-compile",
        check_arg: "",
        why: "start: boot step 1 runs preflight --fix, the step that repairs a broken installation at boot (named, D:730)",
        level: "real-level",
    },
    ProofAnchor {
        proof: "src/bin/cysd/schedule.rs",
        literal: "$pk/bin/cys-dept",
        rel: "bin/cys-dept",
        category: "start-script",
        check: "sh-syntax",
        check_arg: "",
        why: "restore: the departments' start and restore (the daemon's built-in department restore jobs run it) (named)",
        level: "real-level",
    },
    ProofAnchor {
        proof: "src/bin/cys.rs",
        literal: "cys::pack::pack_dir().join(\"agents.json\")",
        rel: "agents.json",
        category: "config",
        check: "none-prerehearsal",
        check_arg: "json-parse",
        why: "config: read at start; load_agent_spec fails hard on a parse error, so no seat launches (named, D:731; C4 4.12: approved first, post-checked)",
        level: "real-level",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_formation.py",
        literal: "node = os.path.join(PACK_DIR, \"bin\", \"javis_boot_node.py\")",
        rel: "bin/javis_boot_node.py",
        category: "start-script",
        check: "py-compile",
        check_arg: "",
        why: "start: every seat is attached through it — without seats boot step 5 fails on a real machine (RULINGS AL1)",
        level: "real-level",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/cys-dept",
        literal: "fm=\"${CYS_PACK_DIR:-$PACK_DEFAULT}/bin/javis_formation.py\"",
        rel: "bin/javis_formation.py",
        category: "start-script",
        check: "py-compile",
        check_arg: "",
        why: "start: the standing formation attaches every department seat (cys-dept launch → formation → boot_node) (RULINGS AL1)",
        level: "real-level",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_formation.py",
        literal: "        import javis_cli_probe",
        rel: "bin/javis_cli_probe.py",
        category: "start-dep",
        check: "py-compile",
        check_arg: "",
        why: "start: formation decides the installed CLIs with it — all 'absent' is pending-cli (no seat), a syntax error escapes `except ImportError` and ends ensure (formation 488-496 · 1670-1678); a stop branch only the seat-attach level reaches (RULINGS AN2)",
        level: "real-level",
    },
    // ── the sibling protection list (C4 4.10 · RULINGS AI6) ──
    ProofAnchor {
        proof: "src/pack.rs",
        literal: "SYSTEM_LOCKED: &[&str] = &[\"trusted-keys.json\"]",
        rel: "trusted-keys.json",
        category: "protection",
        check: "json-parse",
        check_arg: "",
        why: "protection 1 (update signature check): the keyring",
        level: "",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_preflight.py",
        literal: "CAPGATE_HOOK = (\"role-capability-gate.sh\"",
        rel: "hooks/role-capability-gate.sh",
        category: "protection",
        check: "self-test",
        check_arg: "--self-test",
        why: "protections 3 and 4 (reviewers cannot modify review targets; CSO file write limit)",
        level: "",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_preflight.py",
        literal: "\"hooks/guard.sh\"",
        rel: "hooks/guard.sh",
        category: "protection",
        check: "sh-syntax",
        check_arg: "",
        why: "protection 2 (owner approval before irreversible actions / external sends): the irreversible-action gate",
        level: "",
    },
    // ── RULINGS AP2: the data files whose CONTENT decides whether a protection hook is registered ──
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_preflight.py",
        literal: "for path in (base, base + \".example\"):",
        rel: "state/hook-targets.json.example",
        category: "protection",
        check: "json-parse",
        check_arg: "",
        why: "protections 3 and 4 — registration: the shipped fallback target table; C28 unregisters the capability gate from every profile it marks capgate=deny, whatever the daemon says (preflight 596-637 · 4553-4560); javis_guard_register.py reads the same table to register guard.sh (RULINGS AP2)",
        level: "",
    },
    ProofAnchor {
        proof: "cysjavis-pack/bin/javis_preflight.py",
        literal: "import javis_guard_register as _gr",
        rel: "bin/javis_guard_register.py",
        category: "protection",
        check: "py-compile",
        check_arg: "",
        why: "protections 3 and 4 — the registrar's decision path: at every boot C28 calls its validate_targets_doc on the target table to decide whether the capability gate is registered (preflight 581-594 · 596-637); also the manual registrar of guard.sh (RULINGS AW2 — corrects AN7 for this file)",
        level: "",
    },
];

/// RULINGS AP2 — protection rows by PATH RULE: operator files the pack does not ship, which are
/// protection-related in every lane's pack when they exist.
pub const PROTECTION_PATH_RULES: &[ProofAnchor] = &[ProofAnchor {
    proof: "cysjavis-pack/bin/javis_preflight.py",
    literal: "base = os.path.join(pack, \"state\", \"hook-targets.json\")",
    rel: "state/hook-targets.json",
    category: "protection",
    check: "json-parse",
    check_arg: "",
    why: "protections 3 and 4 — registration: the operator's target table (not shipped; read before the .example); C28 unregisters the capability gate from every profile it marks capgate=deny (preflight 596-637 · 4553-4560); javis_guard_register.py registers guard.sh by it (RULINGS AP2)",
    level: "",
}];

/// Every `def _register*` / `def _unregister*` the preflight may define (the two MCP registrars
/// are not hook registrars; `_unregister_event_hook` only removes).
pub const KNOWN_PREFLIGHT_DEFS: &[&str] = &[
    "_register_hook",
    "_register_statusline",
    "_register_mcp",
    "_register_nlm_mcp",
    "_register_appbuild_hook",
    "_register_event_hook",
    "_unregister_event_hook",
];

/// The hook registrar call sites of the preflight, each EXACTLY as often as known (head = name +
/// first args; RULINGS AI2 — the strictness is kept: a count that differs fails the build). T2's
/// M11 narrow mode `--reconcile-hooks` (`reconcile_hooks`) calls the same registrars for the rows of
/// `_reconcile_hook_table` — pinned by [`RECONCILE_TABLE`], built only from the constants this
/// derivation already reads (no new hook, P1).
pub const KNOWN_REGISTRAR_CALLS: &[(&str, &str, usize)] = &[
    (
        "self._register_hook(t)",
        "C08 session-start.sh; M11 reconcile row c08",
        2,
    ),
    (
        "self._register_statusline(t)",
        "C32 statusline (excluded, lifeline); M11 reconcile row c32",
        2,
    ),
    (
        "self._register_event_hook(t, ev, self.EVENT_HOOK",
        "C27 EVENT_HOOK",
        1,
    ),
    (
        "self._register_appbuild_hook(t, respect_tombstone=True)",
        "C33 APPBUILD_HOOK",
        1,
    ),
    (
        "self._register_event_hook(t, _ev, CAPGATE_HOOK[0]",
        "capability gate CAPGATE_HOOK",
        1,
    ),
    (
        "self._register_event_hook(t, event, script_name",
        "C28 loop over SELFCORR_HOOKS",
        1,
    ),
    (
        "self._register_event_hook(t, hevent, hname",
        "C55 loop over GRILL_HOOKS",
        1,
    ),
    (
        "self._register_appbuild_hook(t, respect_tombstone=not lifeline)",
        "M11 reconcile row appbuild (APPBUILD_HOOK)",
        1,
    ),
    (
        "self._register_event_hook(t, event, script",
        "M11 reconcile rows event (EVENT_HOOK · SELFCORR_HOOKS · GRILL_HOOKS)",
        1,
    ),
];

/// RULINGS AI2 — T2's reconcile table (`_reconcile_hook_table`, the rows `--reconcile-hooks`
/// registers): every statement that builds a row, squashed. The table must hold exactly these
/// (one `rows = [` and five `rows.append(`) — a row from anywhere else fails the build.
pub const RECONCILE_TABLE: &[&str] = &[
    "rows = [(\"SessionStart\", \"session-start.sh\", True, \"c08\", None, None), (\"statusLine\", \"cys-statusline.sh\", True, \"c32\", None, None)]",
    "for ev in self.EVENT_HOOK_EVENTS: rows.append((ev, self.EVENT_HOOK, False, \"event\", \"\", None))",
    "for script, events in SELFCORR_HOOKS: for ev, m in events: rows.append((ev, script, False, \"event\", m, hook_timeout_for(script, ev)))",
    "for ev, m in CAPGATE_HOOK[1]: rows.append((ev, CAPGATE_HOOK[0], True, \"capgate\", m, hook_timeout_for(CAPGATE_HOOK[0], ev)))",
    "rows.append((\"PreToolUse\", APPBUILD_HOOK, False, \"appbuild\", \"Edit|Write|NotebookEdit\", None))",
    "for name, ev, m in GRILL_HOOKS: rows.append((ev, name, False, \"event\", m, None))",
];

/// Texts that tie the two loop call sites to their constants.
pub const LOOP_ANCHORS: &[&str] = &[
    "_reg_hooks = list(SELFCORR_HOOKS)",
    "for hname, hevent, hmatcher in GRILL_HOOKS:",
];

/// Pack Python files allowed to write a `hooks` table into a settings file (product registrar +
/// the two manual tools; preflight:1389-1398 lists them). Any other writer fails the census.
pub const KNOWN_SETTINGS_WRITERS: &[&str] = &[
    "bin/javis_preflight.py",
    "bin/javis_dept_migrate.py",
    "bin/javis_guard_register.py",
];

/// RULINGS AN3 / AP3 — class-1 rows the structural rule does not find: files a registered hook
/// reaches through a launch idiom, an import of such a file, or a guarded import of a listed hook
/// dependency. (rel, hook, stage-5, record). `stage-5 = false`: measured by the hook replay — the
/// hook's result changes when the file is broken (`isolated_home_start_replay` (3), the recipe
/// named). `stage-5 = true`: no recipe at the replay level both runs the file and makes the
/// hook's result depend on it (AP3 (a)+(b)) → listed and named as a stage-5 item.
pub const MEASURED_HOOK_DEPS: &[(&str, &str, bool, &str)] = &[
    (
        "bin/javis_teamtoken.py",
        "hooks/role-bootstrap.sh",
        false,
        "role-bootstrap.sh runs hooks/teamtoken-issue.sh, which runs it, when a seat's team-token window is open; broken -> the hook's stdout gains the 'cannot judge the approval' notice (recipe teamtoken)",
    ),
    (
        "bin/javis_state_snapshot.py",
        "hooks/role-bootstrap.sh",
        true,
        "javis_teamtoken.py imports it on Windows only (feed_jsonl_path); the macOS hook replay cannot reach it (stage-5)",
    ),
    (
        "hooks/inject_gate.py",
        "hooks/inject-context.sh",
        false,
        "the poison gate on the injected working state (inject-context.sh 266-282); broken -> a line the scanner quarantines is injected verbatim (recipe state-poison)",
    ),
    (
        "bin/javis_skillscan.py",
        "hooks/inject-context.sh",
        false,
        "the scanner inject_gate.py imports; broken -> the gate prints its 'gate down' banner and injects verbatim (recipe state-poison)",
    ),
    (
        "bin/skillscan_rules.json",
        "hooks/inject-context.sh",
        false,
        "the rules javis_skillscan.py reads (a data table, RULINGS BR2): VALID BUT MATCHING NOTHING (a non-empty `rule_specs` whose patterns match nothing) -> it replaces the built-in rules (javis_skillscan.py:192-200) and the poisoned line is injected verbatim instead of quarantined (recipe state-poison, third mode); unparsable or emptied -> the built-in rules apply and nothing in the hook's output differs",
    ),
    (
        "hooks/teamtoken-issue.sh",
        "hooks/role-bootstrap.sh",
        true,
        "the team-token issuer; issuing needs an open ask (a pending team proposal in the daemon feed published by this seat, the mission delivery ledger, a hook witness newer than the ask) — not built at the replay level (stage-5)",
    ),
];

/// RULINGS BR1 — LISTED class-1 rows proven at stage 5 although a replay reads them: what a replay
/// can break of theirs changes only the checklist-report region (a report — BM1), while their
/// output is USED on a path no recipe reaches. (rel, record). Each must be a class-1 row; the hook
/// replay's step (3) records them instead of failing ("listed rows to be proven").
pub const STAGE5_HOOK_ROWS: &[(&str, &str)] = &[
    (
        "bin/javis_detect.py",
        "hooks/role-bootstrap-legacy.sh:531-545 / :633 (`DETECT_VERDICT=$(… javis_detect.py hook-gate)`) uses its answer on the master-declaration path (no recipe reaches it); the state recipes reach it only through preflight, where breaking it changes the checklist's preflight count (FAIL 6 -> 7) and nothing else",
    ),
    (
        "bin/javis_mission.py",
        "hooks/role-bootstrap-legacy.sh:554-625 / :697 (records and judges the mission through it) uses its answer on the master-declaration path (no recipe reaches it); the state recipes reach it only through preflight, where breaking it changes the checklist's preflight count (FAIL 6 -> 7) and nothing else",
    ),
];

/// RULINGS BM2 — drops BY CONSTRUCTION: files a registered hook (or a class-1 file) only LAUNCHES,
/// at sites that discard both the output and the exit status — breaking the file cannot stop or
/// change that hook. (rel, record). The census pins every launch site to a discarding form
/// ([`by_construction_census`]); an import, or a launch in any other form, fails the build naming
/// the file and the line. Each is a stage-5 CONFIRMATION item, not a row.
pub const BY_CONSTRUCTION_DROPS: &[(&str, &str)] = &[(
    "bin/javis_reflect.py",
    "launched only by hooks/reflect-scan.sh:51-52 (`\"$CYS_PY\" … javis_reflect.py … scan … >/dev/null 2>&1`, exit untested, `set +e`); recipe round runs it (with a transcript): both break modes -> the hook's exit and stdout unchanged; boot layer: never read by the boot, broken with every unlisted file -> the boot completes · (ii) no stop branch",
)];

/// RULINGS AL1/AL3/AN2/AN6 — the candidates the census finds one step from a LISTED file that are NOT
/// boot-critical, each with BOTH break modes measured or read: (i) the file cannot run
/// (`test_boot_critical_start_replay` — bootstrap layer, normal and team-deficit scenes, registered
/// hooks checked; the hook replay for the hook-layer candidates of AN3) and (ii) the stop branches
/// of the start path (`javis_bootstrap.py`, `javis_orchestra.py check`, `javis_boot_node.py`,
/// `javis_formation.py`, `cys-dept`) — none obeys this file's answer as a stop. Stated limit (not
/// measured): a file that hangs.
pub const DROPPED_CANDIDATES: &[(&str, &str)] = &[
    ("bin/javis_brief_lint.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597) (C74 checks it exists)"),
    ("bin/javis_channels.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (orchestra's optional import with a fallback) · (ii) no stop branch: `orchestra check` does not consult it (only `channel-health`, orchestra 4858)"),
    ("bin/javis_cleanroom.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_completion_guard.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_cycle_autopilot.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_dept_schedule.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes; cys-dept runs it when a department is CREATED · (ii) no stop branch at start: cys-dept consults it only at department creation (cys-dept 1194)"),
    ("bin/javis_event.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes · (ii) no stop branch: formation emits events best-effort and ignores the answer (formation 1033-1043)"),
    ("bin/javis_manifest.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: `orchestra check` does not consult it (ticket phases only, orchestra 1736)"),
    ("bin/javis_mcpgate.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_memory.py", "(i) read by the boot (preflight's check tool, `_check_bin_tool(cid, \"javis_memory.py\")` preflight:3413); broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes); hook layer (RULINGS BM1): recipes state-poison and state run it through inject-context.sh's checklist phase (preflight) — broken, ONLY the checklist report region (from the line `■ 실측 체크리스트(…)` / `⚠ 실측 체크리스트 미수행(` to the restore note `▶ `) differs: preflight's summary line, `FAIL 6 · WARN 20` -> `FAIL 8 · WARN 19`; the rest of the injected text and the exit are unchanged; recipe round: reflect-scan.sh:57 tests its exit only to append one `.state_log` warning (:58-59) — the hook's exit and stdout unchanged in both modes · (ii) failing answer (prints a line, exit 1): the same difference, only inside the region; no start file obeys its answer as a stop: only preflight consults it at the boot, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_phoenix.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes; cys-dept calls it best-effort (`|| true`) and the daemon restores from its EMBEDDED copy first · (ii) no stop branch: cys-dept ignores its answer (cys-dept 109-134)"),
    ("bin/javis_radio.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_registry.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_report.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_role.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes; no recipe runs it; reached only through javis_snapshot.py's guarded import `_role_mod` (:49-55, try/except — the role falls back to the env) → AL1's optional import (RULINGS BM3) · (ii) no stop branch: none of the five start files consults it"),
    ("bin/javis_route.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (preflight C17's tool, preflight:3254); hook layer (RULINGS BM1): recipes state-poison and state run it through inject-context.sh's checklist phase (preflight) — broken, ONLY the checklist report region (from the line `■ 실측 체크리스트(…)` / `⚠ 실측 체크리스트 미수행(` to the restore note `▶ `) differs: preflight's summary line, `FAIL 6` -> `FAIL 7`; the rest of the injected text and the exit are unchanged; grill_gate.py:201-211 runs it only in `begin` (grill-arm.sh, output discarded) to set the floor of a LATER grill run (BM2's last sentence) · (ii) failing answer (prints a line, exit 1): the same difference, only inside the region; no start file obeys its answer as a stop: only preflight consults it at the boot, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_runtime_seal.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_scrub.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool); hook layer (RULINGS BM1): recipes state-poison and state run it through inject-context.sh's checklist phase (preflight) — broken, ONLY the checklist report region (from the line `■ 실측 체크리스트(…)` / `⚠ 실측 체크리스트 미수행(` to the restore note `▶ `) differs: preflight's summary line, `WARN 20` -> `WARN 21`; the rest of the injected text and the exit are unchanged; javis_snapshot.py:107-113 imports it inside try/except (AL1's optional import) · (ii) failing answer (prints a line, exit 1): the same difference, only inside the region; no start file obeys its answer as a stop: only preflight consults it at the boot, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_semver.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_serena_probe.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_task.py", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes; no recipe runs it; reached only through guarded imports — javis_snapshot.py `_fresh_task_mod` (:67-72, called inside try/except :406-412) and javis_preflight.py C75 (:6946-6950, try/except) → AL1's optional import (RULINGS BM3) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("bin/javis_todo_decl.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (orchestra's import for ticket prompts) · (ii) no stop branch: `orchestra check` does not consult it (task-prompt only, orchestra 1768)"),
    ("bin/javis_verdict.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (orchestra's optional import with a fallback) · (ii) no stop branch: `orchestra check` does not consult it (review commands only)"),
    ("bin/javis_wakeup.py", "(i) read by the boot; broken alone -> the boot completes and the hooks are registered (bootstrap replay, both scenes) (a preflight check's tool); hook layer (RULINGS BM1): recipes state-poison and state run it through inject-context.sh's checklist phase (preflight) — broken, ONLY the checklist report region (from the line `■ 실측 체크리스트(…)` / `⚠ 실측 체크리스트 미수행(` to the restore note `▶ `) differs: preflight's summary line, `WARN 20` -> `WARN 21`; the rest of the injected text and the exit are unchanged; javis_snapshot.py:75-79 / :429-435 imports it inside try/except (AL1's optional import) · (ii) failing answer (prints a line, exit 1): the same difference, only inside the region; no start file obeys its answer as a stop: only preflight consults it at the boot, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
    ("hooks/brief-lint-warn.sh", "(i) never read by the boot (tripwire, both scenes); broken with every unlisted file -> the boot completes; not a hook the product registers (C74 only checks it exists; an owner registers it by hand) · (ii) no stop branch: only preflight consults it, and the boot does not stop on preflight's answer (bootstrap 3594-3597)"),
];

const CATEGORY_ORDER: &[&str] = &[
    "hook",
    "start-script",
    "hook-dep",
    "start-dep",
    "config",
    "protection",
];

/// CRLF-tolerant text (B6).
pub fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// A clear-procedure file (4.3 / F2): session state, todo files. The statusline scripts are
/// excluded separately (they come from the anchors).
pub fn is_clear_procedure_rel(rel: &str) -> bool {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    base.starts_with("SESSION_STATE") || base.ends_with("_TODO.md") || base == "TODO.md"
}

// ─────────────────────────────────────────────────────────────────────────────
// A small reader of Python literal expressions (strings, names, numbers, tuples, lists, dicts,
// calls such as `os.path.join("hooks", "x")`). Comments and newlines between tokens are skipped.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Py {
    Str(String),
    Name(String),
    Num(i64),
    Seq(Vec<Py>),
    Dict(Vec<(Py, Py)>),
    Call(String, Vec<Py>),
}

impl Py {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Py::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Py] {
        match self {
            Py::Seq(v) => v,
            _ => &[],
        }
    }
}

struct PyReader<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> PyReader<'a> {
    fn skip(&mut self) {
        while self.i < self.s.len() {
            match self.s[self.i] {
                b' ' | b'\t' | b'\n' | b'\r' | b'\\' => self.i += 1,
                b'#' => {
                    while self.i < self.s.len() && self.s[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
    fn string(&mut self) -> Result<String, String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b'\\' && self.i + 1 < self.s.len() {
                out.push(self.s[self.i + 1]);
                self.i += 2;
                continue;
            }
            if c == q {
                self.i += 1;
                return String::from_utf8(out).map_err(|_| "non-UTF-8 string literal".to_string());
            }
            if c == b'\n' {
                break;
            }
            out.push(c);
            self.i += 1;
        }
        Err("unterminated string literal".to_string())
    }
    fn seq(&mut self, close: u8) -> Result<Vec<Py>, String> {
        self.i += 1;
        let mut v = Vec::new();
        loop {
            match self.peek() {
                Some(c) if c == close => {
                    self.i += 1;
                    return Ok(v);
                }
                Some(b',') => self.i += 1,
                Some(_) => v.push(self.value()?),
                None => return Err("unterminated bracket".to_string()),
            }
        }
    }
    fn value(&mut self) -> Result<Py, String> {
        match self.peek() {
            Some(b'"') | Some(b'\'') => {
                let mut s = self.string()?;
                // implicit concatenation of adjacent literals
                while matches!(self.peek(), Some(b'"') | Some(b'\'')) {
                    s.push_str(&self.string()?);
                }
                Ok(Py::Str(s))
            }
            Some(b'[') => Ok(Py::Seq(self.seq(b']')?)),
            Some(b'(') => Ok(Py::Seq(self.seq(b')')?)),
            Some(b'{') => {
                self.i += 1;
                let mut d = Vec::new();
                loop {
                    match self.peek() {
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Py::Dict(d));
                        }
                        Some(b',') => self.i += 1,
                        Some(_) => {
                            let k = self.value()?;
                            if self.peek() != Some(b':') {
                                return Err("dict key without ':'".to_string());
                            }
                            self.i += 1;
                            let v = self.value()?;
                            d.push((k, v));
                        }
                        None => return Err("unterminated dict".to_string()),
                    }
                }
            }
            Some(c) if c.is_ascii_digit() || c == b'-' => {
                let st = self.i;
                self.i += 1;
                while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[st..self.i]).unwrap_or("x");
                t.parse::<i64>()
                    .map(Py::Num)
                    .map_err(|_| format!("bad number {t:?}"))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let st = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric()
                        || self.s[self.i] == b'_'
                        || self.s[self.i] == b'.')
                {
                    self.i += 1;
                }
                let name = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                if self.s.get(self.i) == Some(&b'(') {
                    let args = self.seq(b')')?;
                    return Ok(Py::Call(name, args));
                }
                Ok(Py::Name(name))
            }
            Some(c) => Err(format!("unexpected character {:?}", c as char)),
            None => Err("unexpected end of text".to_string()),
        }
    }
}

/// Parses the Python expression that starts at byte `at` of `text`.
pub fn parse_py_at(text: &str, at: usize) -> Result<Py, String> {
    let mut r = PyReader {
        s: text.as_bytes(),
        i: at,
    };
    r.value()
}

/// Byte offset just after `NAME =` of the ONE column-0 assignment of `name` (module constant).
pub fn py_module_assign(text: &str, name: &str) -> Result<usize, String> {
    let mut hits = Vec::new();
    let mut off = 0usize;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix(name) {
            let t = rest.trim_start_matches([' ', '\t']);
            if t.starts_with('=') && !t.starts_with("==") {
                let eq = line.len() - t.len();
                hits.push(off + eq + 1);
            }
        }
        off += line.len();
    }
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(format!("module constant `{name}` not found")),
        n => Err(format!(
            "module constant `{name}` assigned {n} times (ambiguous)"
        )),
    }
}

/// Like [`py_module_assign`] for an INDENTED (class) constant: exactly one line `<indent>NAME = …`.
pub fn py_class_assign(text: &str, name: &str) -> Result<usize, String> {
    let mut hits = Vec::new();
    let mut off = 0usize;
    for line in text.split_inclusive('\n') {
        let body = line.trim_start_matches([' ', '\t']);
        if body.len() < line.len() {
            if let Some(rest) = body.strip_prefix(name) {
                let t = rest.trim_start_matches([' ', '\t']);
                if t.starts_with('=') && !t.starts_with("==") {
                    hits.push(off + (line.len() - t.len()) + 1);
                }
            }
        }
        off += line.len();
    }
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(format!("class constant `{name}` not found")),
        n => Err(format!(
            "class constant `{name}` assigned {n} times (ambiguous)"
        )),
    }
}

fn py_const(text: &str, name: &str) -> Result<Py, String> {
    let at = py_module_assign(text, name)?;
    parse_py_at(text, at).map_err(|e| format!("module constant `{name}`: {e}"))
}

/// Resolves a `Py::Name` through a module string constant (`GRILL_HOOK = "grill-gate.sh"`).
fn py_resolve_str(text: &str, v: &Py) -> Result<String, String> {
    match v {
        Py::Str(s) => Ok(s.clone()),
        Py::Name(n) => match py_const(text, n)? {
            Py::Str(s) => Ok(s),
            other => Err(format!("`{n}` is not a string constant ({other:?})")),
        },
        other => Err(format!(
            "expected a string or a constant name, found {other:?}"
        )),
    }
}

/// The body of `def NAME(` (up to the next def / decorator / class at the same or a lower indent).
pub fn py_def_body<'t>(text: &'t str, name: &str) -> Result<&'t str, String> {
    let needle = format!("def {name}(");
    let hits: Vec<usize> = text.match_indices(&needle).map(|(i, _)| i).collect();
    if hits.len() != 1 {
        return Err(format!(
            "`def {name}(` found {} times (expected once)",
            hits.len()
        ));
    }
    let st = hits[0];
    let line_start = text[..st].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let indent = &text[line_start..st];
    let rest = &text[st..];
    let mut end = rest.len();
    for marker in ["def ", "@", "class "] {
        let pat = format!("\n{indent}{marker}");
        if let Some(p) = rest[1..].find(&pat) {
            end = end.min(p + 1);
        }
    }
    Ok(&rest[..end])
}

/// The quoted text right after `prefix` (`prefix` ends just before the opening quote).
fn quoted_after(text: &str, prefix: &str) -> Option<String> {
    let at = text.find(prefix)? + prefix.len();
    let rest = &text[at..];
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let body = &rest[1..];
    Some(body[..body.find(q)?].to_string())
}

/// The event a registrar body writes (`.setdefault("hooks", {}).setdefault("<Event>"`).
fn registrar_event(body: &str) -> Option<String> {
    quoted_after(body, ".setdefault(\"hooks\", {}).setdefault(")
}

// ─────────────────────────────────────────────────────────────────────────────
// Rust anchors
// ─────────────────────────────────────────────────────────────────────────────

/// Production text of a Rust source (everything before `#[cfg(test)]\nmod tests {`).
pub fn rust_production(text: &str) -> &str {
    match text.find("#[cfg(test)]\nmod tests {") {
        Some(p) => &text[..p],
        None => text,
    }
}

fn strip_rust_line_comments(s: &str) -> String {
    s.lines()
        .map(|l| match l.find("//") {
            Some(p) => &l[..p],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_u64_const(text: &str, name: &str) -> Result<u32, String> {
    let needle = format!("pub const {name}: u64 = ");
    let at = text
        .find(&needle)
        .ok_or_else(|| format!("const `{name}` not found in {PACK_RS}"))?
        + needle.len();
    let digits: String = text[at..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse::<u32>()
        .map_err(|_| format!("const `{name}` is not a number"))
}

/// One registration read from an anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub script: String,
    pub event: String,
    pub timeout: Option<u32>,
    pub source: &'static str,
}

/// `src/pack.rs` `AWAKENING_HOOKS` → registrations (script, event, timeout).
pub fn awakening_hooks(pack_rs: &str) -> Result<Vec<Registration>, String> {
    let head = "pub const AWAKENING_HOOKS: [DesiredHook; ";
    let at = pack_rs
        .find(head)
        .ok_or_else(|| format!("anchor {PACK_RS} `AWAKENING_HOOKS` not found"))?;
    let after = &pack_rs[at + head.len()..];
    let n: usize = after
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .map_err(|_| format!("anchor {PACK_RS} `AWAKENING_HOOKS`: array length unreadable"))?;
    let body_start = after
        .find("= [")
        .ok_or_else(|| format!("anchor {PACK_RS} `AWAKENING_HOOKS`: no `= [`"))?;
    let body_end = after
        .find("\n];")
        .ok_or_else(|| format!("anchor {PACK_RS} `AWAKENING_HOOKS`: no closing `];`"))?;
    let body = strip_rust_line_comments(&after[body_start..body_end]);
    let mut out = Vec::new();
    for blk in body.split("DesiredHook {").skip(1) {
        let blk = &blk[..blk.find('}').unwrap_or(blk.len())];
        let script = quoted_after(blk, "script: ").ok_or_else(|| {
            format!("anchor {PACK_RS} `AWAKENING_HOOKS`: an entry without `script`")
        })?;
        let event = quoted_after(blk, "event: ").ok_or_else(|| {
            format!("anchor {PACK_RS} `AWAKENING_HOOKS`: `{script}` without `event`")
        })?;
        let t_at = blk.find("timeout:").ok_or_else(|| {
            format!("anchor {PACK_RS} `AWAKENING_HOOKS`: `{script}` without `timeout`")
        })?;
        let t = blk[t_at + "timeout:".len()..].trim_start();
        let timeout = if t.starts_with("None") {
            None
        } else if let Some(inner) = t.strip_prefix("Some(") {
            let inner = &inner[..inner.find(')').unwrap_or(inner.len())];
            let inner = inner.trim();
            if inner.chars().all(|c| c.is_ascii_digit()) {
                Some(
                    inner
                        .parse::<u32>()
                        .map_err(|_| "timeout literal".to_string())?,
                )
            } else {
                Some(rust_u64_const(pack_rs, inner)?)
            }
        } else {
            return Err(format!(
                "anchor {PACK_RS} `AWAKENING_HOOKS`: `{script}` timeout unreadable"
            ));
        };
        out.push(Registration {
            script,
            event,
            timeout,
            source: "src/pack.rs AWAKENING_HOOKS",
        });
    }
    if out.len() != n {
        return Err(format!(
            "anchor {PACK_RS} `AWAKENING_HOOKS`: declared {n} entries, read {}",
            out.len()
        ));
    }
    Ok(out)
}

/// `src/agy_statusline.rs` `SCRIPT` / `SCRIPT_CMD` (excluded statusline hooks, O-C4-4).
pub fn agy_statusline_scripts(agy_rs: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for c in ["SCRIPT", "SCRIPT_CMD"] {
        let pre = format!("pub const {c}: &str = ");
        out.push(
            quoted_after(agy_rs, &pre).ok_or_else(|| format!("anchor {AGY_RS} `{c}` not found"))?,
        );
    }
    Ok(out)
}

/// `src/repair/boot_critical.rs` `LIFELINE_HOOKS` (the one lifeline list, C8 O-C8-6).
pub fn lifeline_hooks(src: &str) -> Result<Vec<String>, String> {
    let head = "pub const LIFELINE_HOOKS: [&str; ";
    let at = src
        .find(head)
        .ok_or_else(|| format!("anchor {LIFELINE_RS} `LIFELINE_HOOKS` not found"))?;
    let after = &src[at..];
    let st = after.find("= [").ok_or("LIFELINE_HOOKS: no `= [`")? + 3;
    let en = after.find("];").ok_or("LIFELINE_HOOKS: no `];`")?;
    let body = &after[st..en];
    let v: Vec<String> = body
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    if v.is_empty() {
        return Err(format!("anchor {LIFELINE_RS} `LIFELINE_HOOKS` is empty"));
    }
    Ok(v)
}

// ─────────────────────────────────────────────────────────────────────────────
// Preflight anchors
// ─────────────────────────────────────────────────────────────────────────────

/// The registrations the preflight writes automatically, its timeout table, its hook bodies and
/// the statusline script it registers (excluded).
#[derive(Debug, Clone, Default)]
pub struct PreflightAnchors {
    pub registrations: Vec<Registration>,
    pub timeouts: BTreeMap<(String, String), u32>,
    /// (rel, owner script)
    pub hook_bodies: Vec<(String, String)>,
    pub statusline: Vec<String>,
}

fn events_of(v: &Py, what: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for ev in v.items() {
        let first = ev
            .items()
            .first()
            .and_then(Py::as_str)
            .ok_or_else(|| format!("{what}: an event entry is not (\"Event\", …)"))?;
        out.push(first.to_string());
    }
    if out.is_empty() {
        return Err(format!("{what}: no event"));
    }
    Ok(out)
}

pub fn preflight_anchors(pf: &str) -> Result<PreflightAnchors, String> {
    let mut a = PreflightAnchors::default();
    let ctx = |e: String| format!("anchor {PREFLIGHT}: {e}");

    // C28 — SELFCORR_HOOKS = [(script, [(event, matcher), …]), …]
    let sc = py_const(pf, "SELFCORR_HOOKS").map_err(ctx)?;
    if sc.items().is_empty() {
        return Err(ctx("`SELFCORR_HOOKS` is empty or not a list".into()));
    }
    for item in sc.items() {
        let parts = item.items();
        let script = parts
            .first()
            .and_then(Py::as_str)
            .ok_or_else(|| ctx("`SELFCORR_HOOKS` entry without a script name".into()))?;
        let evs = parts
            .get(1)
            .ok_or_else(|| ctx(format!("`SELFCORR_HOOKS` `{script}` without events")))?;
        for ev in events_of(evs, "SELFCORR_HOOKS").map_err(ctx)? {
            a.registrations.push(Registration {
                script: script.to_string(),
                event: ev,
                timeout: None,
                source: "javis_preflight.py SELFCORR_HOOKS (C28)",
            });
        }
    }

    // capability gate — CAPGATE_HOOK = ("role-capability-gate.sh", [("PreToolUse", None)])
    let cg = py_const(pf, "CAPGATE_HOOK").map_err(ctx)?;
    let cg_script = cg
        .items()
        .first()
        .and_then(Py::as_str)
        .ok_or_else(|| ctx("`CAPGATE_HOOK` has no script name".into()))?
        .to_string();
    let cg_events = cg
        .items()
        .get(1)
        .ok_or_else(|| ctx("`CAPGATE_HOOK` has no events".into()))?;
    for ev in events_of(cg_events, "CAPGATE_HOOK").map_err(ctx)? {
        a.registrations.push(Registration {
            script: cg_script.clone(),
            event: ev,
            timeout: None,
            source: "javis_preflight.py CAPGATE_HOOK (conditional)",
        });
    }

    // C33 — APPBUILD_HOOK and the event its registrar writes
    let ab = py_resolve_str(pf, &Py::Name("APPBUILD_HOOK".into())).map_err(ctx)?;
    let ab_body = py_def_body(pf, "_register_appbuild_hook").map_err(ctx)?;
    if !ab_body.contains("_cys_hook_cmd(APPBUILD_HOOK)") {
        return Err(ctx(
            "`_register_appbuild_hook` no longer registers `APPBUILD_HOOK`".into(),
        ));
    }
    let ab_ev = registrar_event(ab_body)
        .ok_or_else(|| ctx("`_register_appbuild_hook`: event not found".into()))?;
    a.registrations.push(Registration {
        script: ab,
        event: ab_ev,
        timeout: None,
        source: "javis_preflight.py APPBUILD_HOOK (C33)",
    });

    // C55 — GRILL_HOOKS = ((NAME, EVENT, MATCHER), …) of module string constants
    let gh = py_const(pf, "GRILL_HOOKS").map_err(ctx)?;
    if gh.items().is_empty() {
        return Err(ctx("`GRILL_HOOKS` is empty".into()));
    }
    for item in gh.items() {
        let parts = item.items();
        if parts.len() < 2 {
            return Err(ctx(
                "`GRILL_HOOKS` entry is not (hook, event, matcher)".into()
            ));
        }
        let script = py_resolve_str(pf, &parts[0]).map_err(ctx)?;
        let event = py_resolve_str(pf, &parts[1]).map_err(ctx)?;
        a.registrations.push(Registration {
            script,
            event,
            timeout: None,
            source: "javis_preflight.py GRILL_HOOKS (C55)",
        });
    }

    // C27 — class constants EVENT_HOOK / EVENT_HOOK_EVENTS
    let eh_at = py_class_assign(pf, "EVENT_HOOK").map_err(ctx)?;
    let eh = parse_py_at(pf, eh_at).map_err(|e| ctx(format!("`EVENT_HOOK`: {e}")))?;
    let eh = eh
        .as_str()
        .ok_or_else(|| ctx("`EVENT_HOOK` is not a string".into()))?
        .to_string();
    let ehe_at = py_class_assign(pf, "EVENT_HOOK_EVENTS").map_err(ctx)?;
    let ehe = parse_py_at(pf, ehe_at).map_err(|e| ctx(format!("`EVENT_HOOK_EVENTS`: {e}")))?;
    if ehe.items().is_empty() {
        return Err(ctx("`EVENT_HOOK_EVENTS` is empty".into()));
    }
    for ev in ehe.items() {
        let ev = ev
            .as_str()
            .ok_or_else(|| ctx("`EVENT_HOOK_EVENTS` holds a non-string".into()))?;
        a.registrations.push(Registration {
            script: eh.clone(),
            event: ev.to_string(),
            timeout: None,
            source: "javis_preflight.py EVENT_HOOK (C27)",
        });
    }

    // C08 — `_register_hook` literal script + event
    let c08 = py_def_body(pf, "_register_hook").map_err(ctx)?;
    let c08_script = quoted_after(c08, "_cys_hook_cmd(")
        .ok_or_else(|| ctx("`_register_hook`: `_cys_hook_cmd(\"…\")` literal not found".into()))?;
    let c08_ev =
        registrar_event(c08).ok_or_else(|| ctx("`_register_hook`: event not found".into()))?;
    a.registrations.push(Registration {
        script: c08_script,
        event: c08_ev,
        timeout: None,
        source: "javis_preflight.py _register_hook (C08)",
    });

    // C32 — the statusline script (excluded from the list, lifeline under C8)
    let c32 = py_def_body(pf, "_register_statusline").map_err(ctx)?;
    a.statusline.push(
        quoted_after(c32, "_cys_hook_cmd(").ok_or_else(|| {
            ctx("`_register_statusline`: `_cys_hook_cmd(\"…\")` not found".into())
        })?,
    );

    // B3 — the product's timeout table
    match py_const(pf, "HOOK_TIMEOUT_S").map_err(ctx)? {
        Py::Dict(d) => {
            for (k, v) in d {
                let ks = k.items();
                let (Some(s), Some(e)) = (
                    ks.first().and_then(Py::as_str),
                    ks.get(1).and_then(Py::as_str),
                ) else {
                    return Err(ctx("`HOOK_TIMEOUT_S` key is not (script, event)".into()));
                };
                let Py::Num(n) = v else {
                    return Err(ctx(format!("`HOOK_TIMEOUT_S[{s}]` is not a number")));
                };
                let n = u32::try_from(n).map_err(|_| ctx("negative timeout".into()))?;
                a.timeouts.insert((s.to_string(), e.to_string()), n);
            }
        }
        _ => return Err(ctx("`HOOK_TIMEOUT_S` is not a dict".into())),
    }

    // hook bodies — HOOK_BODY_FILES = [(os.path.join("hooks", "x"), "owner.sh"), …]
    for item in py_const(pf, "HOOK_BODY_FILES").map_err(ctx)?.items() {
        let parts = item.items();
        let rel = match parts.first() {
            Some(Py::Call(f, args)) if f == "os.path.join" => args
                .iter()
                .map(|a| a.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .map(|v| v.join("/")),
            Some(Py::Str(s)) => Some(s.replace('\\', "/")),
            _ => None,
        }
        .ok_or_else(|| ctx("`HOOK_BODY_FILES` entry path unreadable".into()))?;
        let owner = parts
            .get(1)
            .and_then(Py::as_str)
            .ok_or_else(|| ctx("`HOOK_BODY_FILES` entry without owner".into()))?;
        a.hook_bodies.push((rel, owner.to_string()));
    }
    Ok(a)
}

/// Collapses runs of whitespace (call-site heads are compared in this form).
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Census of the preflight: defs, registrar call sites, loop anchors.
pub fn preflight_census(pf: &str) -> Vec<String> {
    // RULINGS AI8: documentation and comments are not code — a registrar named in a docstring or a
    // comment is not a call site (line structure is kept, so the reported line numbers hold)
    let code = py_code_view(pf);
    let pf = code.as_str();
    let mut errs = Vec::new();
    let mut seen_defs: BTreeMap<String, usize> = BTreeMap::new();
    let mut heads: Vec<(usize, String)> = Vec::new();
    for (no, line) in pf.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("def ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.starts_with("_register_") || name.starts_with("_unregister_") {
                *seen_defs.entry(name.clone()).or_insert(0) += 1;
                if !KNOWN_PREFLIGHT_DEFS.contains(&name.as_str()) {
                    errs.push(format!(
                        "census {PREFLIGHT}:{}: new registrar `def {name}` outside the anchors — add it to the boot-critical derivation (T5a) or rename",
                        no + 1
                    ));
                }
            }
            continue;
        }
        for name in [
            "_register_hook(",
            "_register_statusline(",
            "_register_appbuild_hook(",
            "_register_event_hook(",
        ] {
            let mut from = 0;
            while let Some(p) = line[from..].find(name) {
                let at = from + p;
                from = at + name.len();
                // a different registrar whose name ends with this one (e.g. `x_register_hook(`)
                let prev = line[..at].chars().last();
                if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                // head = from `self.` (if any) through the first three arguments
                let start = if line[..at].ends_with("self.") {
                    at - 5
                } else {
                    at
                };
                let tail = &pf[pf.lines().take(no).map(|l| l.len() + 1).sum::<usize>() + start..];
                let mut depth = 0i32;
                let mut commas = 0;
                let mut end = tail.len();
                for (i, c) in tail.char_indices() {
                    match c {
                        '(' | '[' => depth += 1,
                        ')' | ']' => {
                            depth -= 1;
                            if depth == 0 {
                                end = i + 1;
                                break;
                            }
                        }
                        ',' if depth == 1 => {
                            commas += 1;
                            if commas == 3 {
                                end = i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                heads.push((no + 1, squash(&tail[..end])));
            }
        }
    }
    for d in [
        "_register_hook",
        "_register_statusline",
        "_register_appbuild_hook",
        "_register_event_hook",
    ] {
        if seen_defs.get(d).copied().unwrap_or(0) != 1 {
            errs.push(format!(
                "anchor {PREFLIGHT}: registrar `def {d}` missing or defined twice"
            ));
        }
    }
    for (head, what, want) in KNOWN_REGISTRAR_CALLS {
        let n = heads.iter().filter(|(_, h)| h == head).count();
        if n != *want {
            errs.push(format!(
                "anchor {PREFLIGHT}: registrar call `{head}` ({what}) found {n} times (expected {want})"
            ));
        }
    }
    // RULINGS AI2 — the reconcile table holds exactly the pinned rows
    match pf.find("    def _reconcile_hook_table(self):") {
        None => errs.push(format!(
            "anchor {PREFLIGHT}: `def _reconcile_hook_table` not found (the reconcile registrar lost its table)"
        )),
        Some(at) => {
            let rest = &pf[at + 1..];
            let end = rest.find("\n    def ").map_or(rest.len(), |e| e + 1);
            let body = squash(&rest[..end]);
            for row in RECONCILE_TABLE {
                if !body.contains(row) {
                    errs.push(format!(
                        "anchor {PREFLIGHT}: `_reconcile_hook_table` no longer builds `{row}`"
                    ));
                }
            }
            let appends = body.matches("rows.append(").count();
            let lists = body.matches("rows = [").count();
            if appends != 5 || lists != 1 {
                errs.push(format!(
                    "census {PREFLIGHT}: `_reconcile_hook_table` builds {lists} list(s) and {appends} appended row kind(s) (expected 1 and 5) — a new registration must be read by the boot-critical derivation (T5a)"
                ));
            }
        }
    }
    for (no, h) in &heads {
        if !KNOWN_REGISTRAR_CALLS.iter().any(|(k, _, _)| k == h) {
            errs.push(format!(
                "census {PREFLIGHT}:{no}: registrar call `{h}` outside the anchors — a new hook registration must be read by the boot-critical derivation (T5a)"
            ));
        }
    }
    for l in LOOP_ANCHORS {
        if !pf.contains(l) {
            errs.push(format!(
                "anchor {PREFLIGHT}: `{l}` not found (the loop registrar lost its constant)"
            ));
        }
    }
    errs
}

/// Census of every pack Python file that writes a `hooks` table into a settings file.
pub fn settings_writer_census(src: &Sources) -> Vec<String> {
    let mut errs = Vec::new();
    for rel in src.pack_rels.iter().filter(|r| r.ends_with(".py")) {
        let Some(t) = (src.pack_file)(rel) else {
            continue;
        };
        let t = py_code_view(&lf(&t));
        if t.contains("setdefault(\"hooks\"") && !KNOWN_SETTINGS_WRITERS.contains(&rel.as_str()) {
            errs.push(format!(
                "census: `{rel}` writes a settings `hooks` table — a registrar outside the anchors (T5a derivation)"
            ));
        }
    }
    errs
}

/// Census of the Rust hook writer: every `merge_desired_hooks(` call in `src/pack.rs` production
/// text passes `AWAKENING_HOOKS`, and `DesiredHook {` literals are only the array's.
pub fn rust_census(pack_rs: &str, declared: usize) -> Vec<String> {
    let mut errs = Vec::new();
    let prod = rust_production(pack_rs);
    let mut literals = 0usize;
    for (no, line) in prod.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        if t.contains("DesiredHook {") && !t.contains("struct DesiredHook") {
            literals += 1;
        }
        if let Some(p) = line.find("merge_desired_hooks(") {
            if line[..p].trim_end().ends_with("fn") {
                continue;
            }
            let call = &line[p..];
            let mut depth = 0i32;
            let mut end = call.len();
            for (i, c) in call.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if !call[..end].contains("AWAKENING_HOOKS") {
                errs.push(format!(
                    "census {PACK_RS}:{}: `merge_desired_hooks` called with a hook set other than `AWAKENING_HOOKS`",
                    no + 1
                ));
            }
        }
    }
    if literals != declared {
        errs.push(format!(
            "census {PACK_RS}: {literals} `DesiredHook {{ … }}` literals in production code, `AWAKENING_HOOKS` declares {declared} — a hook set outside the anchors"
        ));
    }
    errs
}

// ─────────────────────────────────────────────────────────────────────────────
// Structural closure (rule 2)
// ─────────────────────────────────────────────────────────────────────────────

fn file_kind_check(rel: &str, text: Option<&str>) -> &'static str {
    if rel.ends_with(".py") {
        return "py-compile";
    }
    if rel.ends_with(".json") {
        return "json-parse";
    }
    if rel.ends_with(".sh") {
        return "sh-syntax";
    }
    let first = text.and_then(|t| t.lines().next()).unwrap_or("");
    if first.starts_with("#!") && first.contains("python") {
        "py-compile"
    } else if first.starts_with("#!") && first.contains("sh") {
        "sh-syntax"
    } else {
        "none-prerehearsal"
    }
}

fn is_shell(rel: &str, text: &str) -> bool {
    rel.ends_with(".sh")
        || text
            .lines()
            .next()
            .is_some_and(|l| l.starts_with("#!") && l.contains("sh") && !l.contains("python"))
}

fn dir_of(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(p) => &rel[..p],
        None => "",
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// The pack file a path-valued shell expression ends with, if any.
fn path_value_target(value: &str, file_dir: &str) -> Option<String> {
    let mut v = value.trim().trim_end_matches('\\').trim();
    for cut in [" 2>", " >", " ||", " &&", " ;", ";", " |"] {
        if let Some(p) = v.find(cut) {
            v = &v[..p];
        }
    }
    let v = v.trim().trim_end_matches(['"', '\'']);
    // a command substitution's OUTPUT is not a path binding (`X="$("$PY" "$PACK/bin/y.py")"`)
    let without_dirname = v.replace("$(dirname", "");
    if without_dirname.contains("$(") || v.contains('`') {
        return None;
    }
    let name_len = v.chars().rev().take_while(|c| is_name_char(*c)).count();
    if name_len == 0 {
        return None;
    }
    let name = &v[v.len() - name_len..];
    let before = &v[..v.len() - name_len];
    if before.ends_with("$(dirname \"$0\")/") || before.ends_with("$(dirname $0)/") {
        return Some(if file_dir.is_empty() {
            name.to_string()
        } else {
            format!("{file_dir}/{name}")
        });
    }
    for d in ["bin/", "hooks/"] {
        if let Some(pre) = before.strip_suffix(d) {
            if pre.is_empty() || pre.ends_with('/') || pre.ends_with('"') || pre.ends_with('}') {
                return Some(format!("{d}{name}"));
            }
        }
    }
    None
}

/// Python source with every comment and every triple-quoted region blanked (newlines kept, so
/// line structure is unchanged); single-line string literals are kept (RULINGS AI8: the census and
/// the import / launch scan never read documentation or comments).
pub fn py_code_view(text: &str) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        // triple-quoted string
        if (c == '"' || c == '\'') && i + 2 < b.len() && b[i + 1] == c && b[i + 2] == c {
            out.push_str("   ");
            i += 3;
            while i < b.len() {
                if b[i] == '\\' && i + 1 < b.len() {
                    out.push(if b[i + 1] == '\n' { '\n' } else { ' ' });
                    out.push(' ');
                    i += 2;
                    continue;
                }
                if b[i] == c && i + 2 < b.len() && b[i + 1] == c && b[i + 2] == c {
                    out.push_str("   ");
                    i += 3;
                    break;
                }
                out.push(if b[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        // single-line string: copied as is
        if c == '"' || c == '\'' {
            out.push(c);
            i += 1;
            while i < b.len() && b[i] != '\n' {
                if b[i] == '\\' && i + 1 < b.len() {
                    out.push(b[i]);
                    out.push(b[i + 1]);
                    i += 2;
                    continue;
                }
                out.push(b[i]);
                i += 1;
                if b[i - 1] == c {
                    break;
                }
            }
            continue;
        }
        // comment
        if c == '#' {
            while i < b.len() && b[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The balanced-parenthesis argument text after `open` (which ends with `(`), or `None`.
fn call_args(text: &str, open_end: usize) -> Option<&str> {
    let mut depth = 1i32;
    for (k, ch) in text[open_end..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[open_end..open_end + k]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The plain string literals of an argument text, in order.
fn string_literals(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b: Vec<char> = args.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let q = b[i];
        if q == '"' || q == '\'' {
            let mut lit = String::new();
            i += 1;
            while i < b.len() && b[i] != q {
                if b[i] == '\\' && i + 1 < b.len() {
                    i += 1;
                }
                lit.push(b[i]);
                i += 1;
            }
            out.push(lit);
        }
        i += 1;
    }
    out
}

/// Files a pack file binds (rule 2 — RULINGS AI1 · AN3): `(rel, how)`.
/// - Python: imports WHEREVER written (also inside `try:` and function bodies), `import_module("x")`
///   / `__import__("x")`, and every sibling tool it LAUNCHES or opens by path:
///   `os.path.join(<…>, "bin", "<x>")`, `os.path.join(<…>, "hooks", "<x>")`, a join whose
///   directory is a variable (the name is looked up beside the file, in `bin/` and in `hooks/`),
///   and `<dir> / "bin" / "<x>"` paths.
/// - Shell: a source line, a variable whose value ends with a pack path, and an interpreter
///   (`python3`, `"$CYS_PY"`, `sh`, `bash`) launching a pack file — in command position, also after
///   `if` / `!` / a pipe and INSIDE `$(…)`; the launched path may be wrapped
///   (`"$(cys_native_path "$PACK/bin/x.py")"`) or built from a directory variable (`"$D/x.sh"`,
///   loops over directories); a path built as `$(cd … && pwd)/x`; file names a `for` loop walks.
pub fn bindings(rel: &str, text: &str, rels: &BTreeSet<&str>) -> Vec<(String, String)> {
    bindings_impl(rel, text, rels, true)
}

/// The STRUCTURAL bindings of a hook (class 1 of RULINGS AL1 — the files the hook uses: a source
/// line, a variable whose value ends with a pack path, a column-0 Python import). The extended
/// rule ([`bindings`]) finds candidates for the census; this one builds the hook-dependency rows.
pub fn hook_bindings(rel: &str, text: &str, rels: &BTreeSet<&str>) -> Vec<(String, String)> {
    bindings_impl(rel, text, rels, false)
}

/// Candidate pack paths for a file NAME whose directory the text does not spell out (a variable or
/// loop directory): a sibling of the file, a `bin/` tool, a hook (RULINGS AN3).
fn by_name(name: &str, file_dir: &str) -> Vec<String> {
    let mut v = vec![if file_dir.is_empty() {
        name.to_string()
    } else {
        format!("{file_dir}/{name}")
    }];
    for d in ["bin", "hooks"] {
        let c = format!("{d}/{name}");
        if !v.contains(&c) {
            v.push(c);
        }
    }
    v
}

/// The pack files a launch argument names (RULINGS AN3): the FIRST `…/bin/x` or `…/hooks/x` in it,
/// or `$VAR/x` / `${VAR}/x` (a directory variable) looked up by name.
fn launch_targets(arg: &str, file_dir: &str) -> Vec<String> {
    let first = ["/bin/", "/hooks/"]
        .iter()
        .filter_map(|d| arg.find(d).map(|p| (p, *d)))
        .min_by_key(|(p, _)| *p);
    if let Some((p, d)) = first {
        let name: String = arg[p + d.len()..]
            .chars()
            .take_while(|c| is_name_char(*c))
            .collect();
        return if name.is_empty() {
            Vec::new()
        } else {
            vec![format!("{}{name}", &d[1..])]
        };
    }
    var_dir_name(arg)
        .map(|n| by_name(&n, file_dir))
        .unwrap_or_default()
}

/// `$VAR/name`, `${VAR}/name`, `${VAR:-…}/name` (quotes stripped) → `name` when it is a script
/// (`.sh` / `.py`).
fn var_dir_name(value: &str) -> Option<String> {
    let w = value.trim().trim_matches(|c| c == '"' || c == '\'');
    if !w.starts_with('$') || w.starts_with("$(") {
        return None;
    }
    let slash = w.rfind('/')?;
    let (dir, name) = (&w[..slash], &w[slash + 1..]);
    let dir_is_one_var = if let Some(inner) = dir.strip_prefix("${") {
        inner.ends_with('}') && !inner[..inner.len() - 1].contains('}')
    } else {
        dir[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    (dir_is_one_var
        && (name.ends_with(".sh") || name.ends_with(".py"))
        && name.chars().all(is_name_char))
    .then(|| name.to_string())
}

fn bindings_impl(
    rel: &str,
    text: &str,
    rels: &BTreeSet<&str>,
    extended: bool,
) -> Vec<(String, String)> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut add = |cand: String, how: String| {
        // documentation never runs: a `.md` file is not a launch / open candidate of the census
        if rels.contains(cand.as_str()) && cand != rel && !(extended && cand.ends_with(".md")) {
            out.entry(cand).or_insert(how);
        }
    };
    let fdir = dir_of(rel);
    if rel.ends_with(".py") || text.lines().next().is_some_and(|l| l.contains("python")) {
        let code = py_code_view(text);
        for line in code.lines() {
            // structural mode: module-level imports only; extended: wherever written
            let t = if extended { line.trim_start() } else { line };
            let mods: Vec<String> = if let Some(r) = t.strip_prefix("import ") {
                r.split(',')
                    .map(|m| m.split_whitespace().next().unwrap_or("").to_string())
                    .collect()
            } else if let Some(r) = t.strip_prefix("from ") {
                if !r.contains(" import") {
                    continue;
                }
                vec![r.split_whitespace().next().unwrap_or("").to_string()]
            } else {
                continue;
            };
            for m in mods {
                if m.is_empty() || m.contains('.') {
                    continue;
                }
                add(format!("bin/{m}.py"), format!("imported by {rel}"));
            }
        }
        if !extended {
            return out.into_iter().collect();
        }
        for opener in ["import_module(", "__import__("] {
            for (at, _) in code.match_indices(opener) {
                if let Some(args) = call_args(&code, at + opener.len()) {
                    if let Some(m) = string_literals(args).first() {
                        add(format!("bin/{m}.py"), format!("imported by {rel}"));
                    }
                }
            }
        }
        let how = format!("launched or opened by path in {rel}");
        for (at, _) in code.match_indices("os.path.join(") {
            if let Some(args) = call_args(&code, at + "os.path.join(".len()) {
                let lits = string_literals(args);
                let Some(last) = lits.last() else { continue };
                if last.contains('/') {
                    let l = last.trim_start_matches("./").to_string();
                    add(l, how.clone());
                    continue;
                }
                if last.is_empty() || !last.chars().all(is_name_char) {
                    continue;
                }
                let prev = lits.len().checked_sub(2).map(|i| lits[i].as_str());
                let cands = match prev {
                    Some("bin") => vec![format!("bin/{last}")],
                    Some("hooks") => vec![format!("hooks/{last}")],
                    // a variable directory: a script is looked up by name, a data file only in bin/
                    _ if last.ends_with(".py") || last.ends_with(".sh") => by_name(last, fdir),
                    _ => vec![format!("bin/{last}")],
                };
                for c in cands {
                    add(c, how.clone());
                }
            }
        }
        // `<dir> / "bin" / "x"` paths
        for d in ["\"bin\"", "\"hooks\"", "'bin'", "'hooks'"] {
            for (at, _) in code.match_indices(d) {
                let rest = code[at + d.len()..].trim_start();
                let Some(rest) = rest.strip_prefix('/') else {
                    continue;
                };
                let rest = rest.trim_start();
                let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                    continue;
                };
                let name: String = rest[1..].chars().take_while(|c| *c != q).collect();
                if !name.is_empty() && name.chars().all(is_name_char) {
                    add(
                        format!("{}/{name}", d.trim_matches(['"', '\''])),
                        how.clone(),
                    );
                }
            }
        }
        return out.into_iter().collect();
    }
    if !is_shell(rel, text) {
        return Vec::new();
    }
    // heredocs: a Python heredoc body (`"$CYS_PY" - <<'PYEOF'`) is read with the Python rules;
    // any other heredoc body is data, not commands (its text is never a launch or a binding)
    let mut heredoc: Option<(String, bool, String)> = None; // (terminator, python, body)
    for line in text.lines() {
        if let Some((term, python, body)) = heredoc.as_mut() {
            if line.trim_end() == term.as_str() {
                if *python {
                    let inner = format!("{rel}#heredoc.py");
                    for (c, how) in bindings_impl(&inner, body, rels, extended) {
                        add(c, how.replace(&inner, rel));
                    }
                }
                heredoc = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(p) = line.find("<<") {
            let after = line[p + 2..].trim_start_matches('-').trim_start();
            let word: String = after
                .trim_start_matches(['\'', '"'])
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !word.is_empty() {
                let python = line[..p].contains("PY") || line[..p].contains("python");
                heredoc = Some((word, python, String::new()));
            }
        }
        if extended {
            // a path built as `$(cd <dir> && pwd)/<name>` (whole line: the `&&` is inside it)
            let mut from = 0;
            while let Some(p) = line[from..].find("pwd)/") {
                let at = from + p;
                let name: String = line[at + 5..]
                    .chars()
                    .take_while(|c| is_name_char(*c))
                    .collect();
                if let Some(cd) = line[..at].rfind("$(cd ") {
                    let dir = &line[cd + 5..at];
                    let cands = if dir.contains("/bin") {
                        vec![format!("bin/{name}")]
                    } else if dir.contains("/hooks") {
                        vec![format!("hooks/{name}")]
                    } else {
                        by_name(&name, fdir)
                    };
                    for c in cands {
                        add(c, format!("launched or opened by path in {rel}"));
                    }
                }
                from = at + 5;
            }
            // file names a `for` loop walks (`for h in a.sh b.sh; do …`)
            if let Some(r) = line.trim_start().strip_prefix("for ") {
                if let Some((_, list)) = r.split_once(" in ") {
                    let list = list.split("; do").next().unwrap_or(list);
                    for w in list.split_whitespace() {
                        let w = w.trim_matches(|c| c == '"' || c == '\'');
                        if !w.contains('$')
                            && !w.contains('/')
                            && (w.ends_with(".sh") || w.ends_with(".py"))
                            && w.chars().all(is_name_char)
                        {
                            for c in by_name(w, fdir) {
                                add(c, format!("launched by {rel} (loop)"));
                            }
                        }
                    }
                }
            }
        }
        let seps: &[&str] = if extended {
            &[" || ", " && ", "; ", " | "]
        } else {
            &[" || ", " && ", "; "]
        };
        let mut segs: Vec<String> = vec![line.to_string()];
        for sep in seps {
            segs = segs
                .iter()
                .flat_map(|s| s.split(sep).map(str::to_string))
                .collect();
        }
        if extended {
            // every `$(` opens a command of its own
            let inner: Vec<String> = segs
                .iter()
                .flat_map(|s| {
                    s.match_indices("$(")
                        .map(|(p, _)| s[p + 2..].to_string())
                        .collect::<Vec<_>>()
                })
                .collect();
            segs.extend(inner);
        }
        for seg in &segs {
            let s = seg
                .trim_start()
                .trim_start_matches("||")
                .trim_start_matches("&&")
                .trim_start();
            // an interpreter launching a pack file — only in COMMAND position (after leading
            // keywords / assignments / exec / env / nohup / command), never inside a quoted message
            let words: Vec<&str> = s.split_whitespace().collect();
            let mut k = 0;
            while k < words.len()
                && (matches!(
                    words[k],
                    "exec"
                        | "env"
                        | "nohup"
                        | "command"
                        | "time"
                        | "if"
                        | "!"
                        | "then"
                        | "do"
                        | "else"
                        | "elif"
                        | "while"
                        | "until"
                        | "{"
                        | "("
                ) || (words[k].contains('=')
                    && words[k].split('=').next().is_some_and(|n| {
                        !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })))
            {
                k += 1;
            }
            let unq = |w: &str| w.trim_matches(|c| c == '"' || c == '\'').to_string();
            let interpreters: &[&str] = if extended {
                &[
                    "python3",
                    "python",
                    "py",
                    "$CYS_PY",
                    "${CYS_PY}",
                    "$PY",
                    "${PY}",
                    "sh",
                    "bash",
                ]
            } else {
                &[]
            };
            if k < words.len() && interpreters.contains(&unq(words[k]).as_str()) {
                let rest = &words[k + 1..];
                if let Some(i) = rest.iter().position(|w| !unq(w).starts_with('-')) {
                    let w0 = rest[i];
                    let arg: String = if w0.trim_start_matches('"').starts_with("$(") {
                        let joined = rest[i..].join(" ");
                        let start = joined.find("$(").map(|p| p + 2).unwrap_or(0);
                        call_args(&joined, start)
                            .map(str::to_string)
                            .unwrap_or_else(|| joined[start..].to_string())
                    } else {
                        w0.to_string()
                    };
                    for t in launch_targets(&arg, fdir) {
                        add(t, format!("launched by {rel}"));
                    }
                }
            }
            let value = if let Some(r) = s.strip_prefix(". ").or_else(|| s.strip_prefix("source "))
            {
                Some(("sourced", r))
            } else {
                let s2 = ["local ", "export ", "readonly ", "declare "]
                    .iter()
                    .find_map(|k| s.strip_prefix(k))
                    .unwrap_or(s);
                let name_len = s2
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .count();
                if name_len > 0
                    && !s2.starts_with(|c: char| c.is_ascii_digit())
                    && s2[name_len..].starts_with('=')
                    && !s2[name_len..].starts_with("==")
                {
                    Some(("bound", &s2[name_len + 1..]))
                } else {
                    None
                }
            };
            let Some((how, v)) = value else { continue };
            if let Some(t) = path_value_target(v, fdir) {
                add(t, format!("{how} by {rel}"));
            } else if extended {
                // a value built from a directory variable (`X="$D/name.sh"` — loops over dirs)
                let mut vv = v.trim();
                for cut in [" 2>", " >", " ;", ";"] {
                    if let Some(p) = vv.find(cut) {
                        vv = &vv[..p];
                    }
                }
                if let Some(n) = var_dir_name(vv) {
                    for c in by_name(&n, fdir) {
                        add(c, format!("{how} by {rel}"));
                    }
                }
            }
        }
    }
    out.into_iter().collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// The derivation
// ─────────────────────────────────────────────────────────────────────────────

/// Everything `root` sources, launches or imports, transitively (the extended rule) — `(rel, how)`,
/// the root itself excluded (RULINGS AT5 / AU1).
pub fn protection_closure(
    src: &Sources,
    rels: &BTreeSet<&str>,
    root: &str,
) -> BTreeMap<String, String> {
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    let mut queue: Vec<String> = vec![root.to_string()];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while let Some(r) = queue.pop() {
        if !seen.insert(r.clone()) {
            continue;
        }
        let text = (src.pack_file)(&r).map(|t| lf(&t)).unwrap_or_default();
        for (c, how) in bindings(&r, &text, rels) {
            if c != root {
                found.entry(c.clone()).or_insert(how);
            }
            queue.push(c);
        }
    }
    found
}

/// RULINGS AU1 / AW2 / BM5 — the protection TESTS the frozen contract names: (id, the files it
/// starts from, follow the protection closure). G (external sends / permanent deletions — the
/// irreversible-action gate), R (reviewer writes) and C (CSO writes — the capability gate): the
/// rows and everything they source, launch or import, transitively (I2 is withdrawn — BC1 / BM5). I3 (registration —
/// the registrar's decision path): the files C28 and its helpers read or call to decide whether a
/// protection hook is registered, enumerated by reading them (the report's registrar table); NOT the
/// transitive closure of `javis_preflight.py` (that is the whole start path).
pub const PROTECTION_TESTS: &[(&str, &[&str], bool)] = &[
    ("C", &["hooks/role-capability-gate.sh"], true),
    ("G", &["hooks/guard.sh"], true),
    (
        "I3",
        &[
            "bin/javis_preflight.py",
            "bin/javis_guard_register.py",
            "bin/javis_lock.py",
            "hooks/role-capability-gate.sh",
            "state/hook-targets.json",
            "state/hook-targets.json.example",
            "directives/CSO_DIRECTIVE.md",
        ],
        false,
    ),
    ("R", &["hooks/role-capability-gate.sh"], true),
];

/// Inputs of a protection test that are RULE DOCUMENTS, not list rows: their content is guarded by
/// the rule-document branch (RULINGS AQ8), so they may stand outside the two lists.
pub const RULE_DOCUMENT_INPUTS: &[&str] = &["directives/CSO_DIRECTIVE.md"];

/// RULINGS AU1 / AW2 — per protection test, the files it reads (sorted). Every file is a
/// protection or a boot-critical row (else an error — AT5), or a declared rule document
/// ([`RULE_DOCUMENT_INPUTS`]), so the daemon's screen can lay out exactly these files.
pub fn protection_inputs(
    src: &Sources,
    entries: &[GenEntry],
) -> Result<Vec<(String, Vec<String>)>, String> {
    let rels: BTreeSet<&str> = src.pack_rels.iter().map(String::as_str).collect();
    let listed: BTreeSet<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
    let mut errs: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (test, roots, follow) in PROTECTION_TESTS {
        let mut files: BTreeSet<String> = BTreeSet::new();
        for r in *roots {
            if !listed.contains(r) && !RULE_DOCUMENT_INPUTS.contains(r) {
                errs.push(format!(
                    "protection test {test}: `{r}` is on neither compiled list and is not a declared rule document (RULINGS AU1 / AW2)"
                ));
                continue;
            }
            files.insert(r.to_string());
            if !*follow {
                continue;
            }
            for (c, how) in protection_closure(src, &rels, r) {
                if !listed.contains(c.as_str()) {
                    errs.push(format!(
                        "protection test {test}: `{c}` ({how}) is on neither compiled list (RULINGS AT5 / AU1)"
                    ));
                }
                files.insert(c);
            }
        }
        out.push((test.to_string(), files.into_iter().collect()));
    }
    if errs.is_empty() {
        Ok(out)
    } else {
        Err(errs.join("\n"))
    }
}

/// Lines of a shell text with `\` continuations joined, comments and heredoc bodies left out:
/// `(first line number, joined text)`.
fn shell_commands(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut heredoc: Option<String> = None;
    let mut cur: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        if let Some(term) = &heredoc {
            if line.trim_end() == term {
                heredoc = None;
            }
            continue;
        }
        if cur.is_none() && line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(p) = line.find("<<") {
            let word: String = line[p + 2..]
                .trim_start_matches('-')
                .trim_start()
                .trim_start_matches(['\'', '"'])
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !word.is_empty() {
                heredoc = Some(word);
            }
        }
        let (n, mut acc) = cur.take().unwrap_or((i + 1, String::new()));
        let body = line.trim_end();
        if let Some(head) = body.strip_suffix('\\') {
            acc.push_str(head);
            acc.push(' ');
            cur = Some((n, acc));
        } else {
            acc.push_str(body);
            out.push((n, acc));
        }
    }
    if let Some(c) = cur {
        out.push(c);
    }
    out
}

/// The command with every file test (`[ -f … ]`, `[ -x … ]`, `[ -e … ]`, `[ -r … ]`) taken out.
fn without_file_tests(cmd: &str) -> String {
    let mut s = cmd.to_string();
    for t in ["[ -f ", "[ -x ", "[ -e ", "[ -r ", "[ -s "] {
        while let Some(a) = s.find(t) {
            let Some(b) = s[a..].find(" ]") else { break };
            s.replace_range(a..a + b + 2, " ");
        }
    }
    s
}

/// RULINGS BM2 — the discarding forms the census knows: a whole simple command (not an `if` /
/// `while` condition, not negated, not `exec`, not inside `$(…)`, no `&&` or pipe) whose first word
/// is an interpreter (optionally after `cys_timeout_run <N>`) and which ends with `>/dev/null 2>&1`,
/// optionally followed by `|| true`, `|| :` or `&`. The file must not run with `set -e`.
pub fn is_discarding_launch(cmd: &str) -> bool {
    let mut t = cmd.trim();
    for tail in [" &", " || true", " || :"] {
        if let Some(h) = t.strip_suffix(tail) {
            t = h.trim_end();
        }
    }
    let Some(head) = t.strip_suffix(">/dev/null 2>&1") else {
        return false;
    };
    if head.contains("&&") || head.contains(" | ") || head.contains("||") {
        return false;
    }
    let mut words = head.split_whitespace();
    let mut first = words.next().unwrap_or("");
    if first == "cys_timeout_run" {
        let _cap = words.next();
        first = words.next().unwrap_or("");
    }
    let first = first.trim_matches(|c| c == '"' || c == '\'');
    matches!(
        first,
        "$CYS_PY" | "${CYS_PY}" | "python3" | "python" | "sh" | "bash"
    )
}

/// Line ranges (1-based, inclusive) of the shell function bodies of a text: from a line
/// `name() {` / `function name {` to the next line that is exactly `}`.
fn function_bodies(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        let opens = t.starts_with("function ")
            || t.split_once("()").is_some_and(|(name, rest)| {
                let name = name.trim();
                !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && rest.trim_start().starts_with('{')
            });
        if opens {
            let end = lines[i + 1..]
                .iter()
                .position(|x| *x == "}")
                .map_or(lines.len(), |p| i + 1 + p + 1);
            out.push((i + 1, end));
        }
    }
    out
}

/// What makes a launch's exit status reach someone after it: the next command (structural words
/// skipped) reads `$?` / `PIPESTATUS` or is a bare `exit` / `return`, or there is no next command
/// (the launch may be the script's last, so its status would be the hook's).
fn exit_status_reader(after: &[(usize, String)]) -> Option<&'static str> {
    let next = after.iter().map(|(_, c)| c.trim()).find(|c| {
        !matches!(
            *c,
            "" | "fi" | "done" | "esac" | ";;" | "}" | "else" | "then" | "do"
        )
    });
    match next {
        None => Some("it may be the script's last command (its status would be the hook's)"),
        Some(c) if c.contains("$?") || c.contains("PIPESTATUS") => {
            Some("the next command reads its exit status")
        }
        Some("exit") | Some("return") => {
            Some("a bare `exit` / `return` follows (its status would be the hook's)")
        }
        Some(_) => None,
    }
}

/// `set -e` (errexit) anywhere in a shell text.
fn has_errexit(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l == "set -e"
            || l.starts_with("set -e ")
            || l.starts_with("set -eu")
            || l.starts_with("set -euo")
            || l.contains("set -o errexit")
    })
}

/// RULINGS BM2 — the census of the by-construction drops over the registered hooks and the class-1
/// files: a Python file that imports or launches one, a shell line naming it (by its file name —
/// a path built through a variable still names it somewhere) in any form but a discarding launch,
/// a shell file that runs with `set -e` (its own text or a pack file it sources), or a drop no
/// launch site names any more → an error naming the file and the line.
pub fn by_construction_census(
    src: &Sources,
    rels: &BTreeSet<&str>,
    class1: &[String],
) -> Vec<String> {
    let mut errs = Vec::new();
    for (drop, _) in BY_CONSTRUCTION_DROPS {
        let name = drop.rsplit('/').next().unwrap_or(drop);
        let mut sites = 0usize;
        for f in class1 {
            let text = (src.pack_file)(f).map(|t| lf(&t)).unwrap_or_default();
            let python =
                f.ends_with(".py") || text.lines().next().is_some_and(|l| l.contains("python"));
            if python {
                if bindings(f, &text, rels).iter().any(|(c, _)| c == drop) {
                    errs.push(format!(
                        "by-construction drop `{drop}`: the class-1 file `{f}` imports or launches it — an import is never a drop by construction (RULINGS BM2)"
                    ));
                }
                continue;
            }
            // errexit in the file itself or in a pack file it sources (`. x` / `source x`)
            let sourced = shell_commands(&text)
                .into_iter()
                .filter(|(_, c)| {
                    let c = c.trim_start();
                    c.starts_with(". ") || c.starts_with("source ")
                })
                .flat_map(|(_, c)| {
                    rels.iter()
                        .filter(|r| {
                            let n = r.rsplit('/').next().unwrap_or(r);
                            n.contains('.') && c.contains(n)
                        })
                        .map(|r| r.to_string())
                        .collect::<Vec<_>>()
                })
                .collect::<BTreeSet<String>>();
            let errexit = has_errexit(&text)
                || sourced.iter().any(|r| {
                    (src.pack_file)(r)
                        .map(|t| has_errexit(&lf(&t)))
                        .unwrap_or(false)
                });
            let cmds = shell_commands(&text);
            let fns = function_bodies(&text);
            for (k, (n, cmd)) in cmds.iter().enumerate() {
                if !without_file_tests(cmd).contains(name) {
                    continue;
                }
                sites += 1;
                let why = if errexit {
                    Some("the file runs with `set -e`")
                } else if !is_discarding_launch(cmd) {
                    Some("not a discarding form")
                } else if fns.iter().any(|(a, b)| a <= n && n <= b) {
                    Some("inside a function body (its status can become the function's)")
                } else {
                    exit_status_reader(&cmds[k + 1..])
                };
                if let Some(why) = why {
                    errs.push(format!(
                        "by-construction drop `{drop}`: {f}:{n} launches it in a form that does not discard both its output and its exit status — {why} (RULINGS BM2): {}",
                        cmd.trim()
                    ));
                }
            }
        }
        if sites == 0 {
            errs.push(format!(
                "by-construction drop `{drop}`: no registered hook or class-1 file launches it any more — remove the entry (RULINGS BM2)"
            ));
        }
    }
    errs
}

pub fn derive(src: &Sources) -> Result<Vec<GenEntry>, String> {
    let rels: BTreeSet<&str> = src.pack_rels.iter().map(String::as_str).collect();
    let need_repo = |p: &str| -> Result<String, String> {
        (src.repo_file)(p)
            .map(|t| lf(&t))
            .ok_or_else(|| format!("anchor file `{p}` unreadable"))
    };
    let pack_rs = need_repo(PACK_RS)?;
    let agy_rs = need_repo(AGY_RS)?;
    let lifeline_src = need_repo(LIFELINE_RS)?;
    let pf = (src.pack_file)(PREFLIGHT)
        .map(|t| lf(&t))
        .ok_or_else(|| format!("anchor file `cysjavis-pack/{PREFLIGHT}` unreadable"))?;

    let mut errs: Vec<String> = Vec::new();
    errs.extend(preflight_census(&pf));
    errs.extend(settings_writer_census(src));
    // anchor readers: every failure is reported together with the census findings
    let awakening = awakening_hooks(&pack_rs).map_err(|e| errs.push(e)).ok();
    if let Some(a) = &awakening {
        errs.extend(rust_census(&pack_rs, a.len()));
    }
    let pfa = preflight_anchors(&pf).map_err(|e| errs.push(e)).ok();
    let agy = agy_statusline_scripts(&agy_rs)
        .map_err(|e| errs.push(e))
        .ok();
    let lifelines = lifeline_hooks(&lifeline_src).map_err(|e| errs.push(e)).ok();
    let (Some(awakening), Some(pfa), Some(agy), Some(lifelines)) = (awakening, pfa, agy, lifelines)
    else {
        return Err(errs.join("\n"));
    };
    let mut excluded: BTreeSet<String> = agy.into_iter().collect();
    excluded.extend(pfa.statusline.iter().cloned());

    // 1. hooks: script → (events, timeout, sources)
    type HookRow = (BTreeSet<String>, Option<u32>, BTreeSet<&'static str>);
    let mut hooks: BTreeMap<String, HookRow> = BTreeMap::new();
    for r in awakening.iter().chain(pfa.registrations.iter()) {
        if excluded.contains(&r.script) {
            continue;
        }
        let e = hooks
            .entry(r.script.clone())
            .or_insert_with(|| (BTreeSet::new(), None, BTreeSet::new()));
        e.0.insert(r.event.clone());
        let declared = r.timeout.or_else(|| {
            pfa.timeouts
                .get(&(r.script.clone(), r.event.clone()))
                .copied()
        });
        if let Some(t) = declared {
            e.1 = Some(e.1.map_or(t, |old: u32| old.max(t)));
        }
        e.2.insert(r.source);
    }
    for ((s, ev), t) in &pfa.timeouts {
        if let Some(e) = hooks.get_mut(s) {
            if e.0.contains(ev) {
                e.1 = Some(e.1.map_or(*t, |old| old.max(*t)));
            }
        } else if !excluded.contains(s) {
            errs.push(format!(
                "anchor {PREFLIGHT} `HOOK_TIMEOUT_S`: `{s}` ({ev}) is not a registered hook"
            ));
        }
    }
    for s in hooks.keys() {
        if !rels.contains(format!("hooks/{s}").as_str()) {
            errs.push(format!(
                "registered hook `{s}` has no file `hooks/{s}` in the pack"
            ));
        }
    }
    for l in &lifelines {
        if !hooks.contains_key(l) && !excluded.contains(l) {
            errs.push(format!(
                "lifeline `{l}` ({LIFELINE_RS}) is neither a registered hook nor an excluded statusline script"
            ));
        }
    }

    let mut rows: BTreeMap<String, GenEntry> = BTreeMap::new();
    for (s, (evs, timeout, sources)) in &hooks {
        let rel = format!("hooks/{s}");
        rows.insert(
            rel.clone(),
            GenEntry {
                rel,
                category: "hook",
                check: "hook-replay",
                check_arg: evs.iter().cloned().collect::<Vec<_>>().join(","),
                timeout_s: *timeout,
                lifeline: lifelines.iter().any(|l| l == s),
                why: format!(
                    "registered: {}",
                    sources.iter().copied().collect::<Vec<_>>().join("; ")
                ),
                proof: "hook-replay",
            },
        );
    }

    // 3/4/5. proof anchors: class 2 / class 3 start rows and the protection list (RULINGS AL1)
    let mut protection: Vec<GenEntry> = Vec::new();
    let anchors = PROOF_ANCHORS
        .iter()
        .map(|a| (a, true))
        .chain(PROTECTION_PATH_RULES.iter().map(|a| (a, false)));
    for (a, shipped) in anchors {
        match (src.repo_file)(a.proof).map(|t| lf(&t)) {
            None => errs.push(format!(
                "proof anchor `{}`: file `{}` unreadable",
                a.rel, a.proof
            )),
            Some(t) if !t.contains(a.literal) => errs.push(format!(
                "proof anchor `{}`: `{}` no longer contains `{}`",
                a.rel, a.proof, a.literal
            )),
            Some(_) => {}
        }
        if shipped && !rels.contains(a.rel) {
            errs.push(format!("proof anchor `{}`: not a file of the pack", a.rel));
            continue;
        }
        if !shipped && a.category != "protection" {
            errs.push(format!(
                "path rule `{}`: only protection rows may name a file the pack does not ship (RULINGS AP2)",
                a.rel
            ));
            continue;
        }
        let row = GenEntry {
            rel: a.rel.to_string(),
            category: a.category,
            check: a.check,
            check_arg: a.check_arg.to_string(),
            timeout_s: None,
            lifeline: false,
            why: format!("{} (proof: {})", a.why, a.proof),
            proof: a.level,
        };
        if a.category == "protection" {
            protection.push(row);
            continue;
        }
        match rows.get(a.rel) {
            Some(existing) if existing.category == "hook" => {}
            _ => {
                rows.insert(a.rel.to_string(), row);
            }
        }
    }

    // 2. class 1: the files the hooks use — STRUCTURAL closure (source lines, path-valued
    //    variables, HOOK_BODY_FILES, module-level imports), each row proven by the hook replay
    type Binder = fn(&str, &str, &BTreeSet<&str>) -> Vec<(String, String)>;
    let walk = |roots: Vec<(String, String)>, bind: Binder| -> Vec<(String, String)> {
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        let mut queue: Vec<(String, String)> = roots;
        while let Some((r, why)) = queue.pop() {
            if seen.contains_key(&r) {
                continue;
            }
            let text = (src.pack_file)(&r).map(|t| lf(&t)).unwrap_or_default();
            for (child, how) in bind(&r, &text, &rels) {
                if !seen.contains_key(&child) {
                    queue.push((child, how));
                }
            }
            seen.insert(r, why);
        }
        seen.into_iter().collect()
    };
    let mut hook_roots: Vec<(String, String)> = hooks
        .keys()
        .map(|s| (format!("hooks/{s}"), "hook".to_string()))
        .collect();
    for (rel, owner) in &pfa.hook_bodies {
        if hooks.contains_key(owner) {
            if !rels.contains(rel.as_str()) {
                errs.push(format!(
                    "anchor {PREFLIGHT} `HOOK_BODY_FILES`: `{rel}` is not in the pack"
                ));
                continue;
            }
            hook_roots.push((
                rel.clone(),
                format!("hook body of {owner} (HOOK_BODY_FILES)"),
            ));
        }
    }
    for (rel, why) in walk(hook_roots, hook_bindings) {
        if rows.contains_key(&rel) {
            continue;
        }
        let base = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        if excluded.contains(&base) || is_clear_procedure_rel(&rel) {
            continue;
        }
        let text = (src.pack_file)(&rel).map(|t| lf(&t));
        rows.insert(
            rel.clone(),
            GenEntry {
                check: file_kind_check(&rel, text.as_deref()),
                rel,
                category: "hook-dep",
                check_arg: String::new(),
                timeout_s: None,
                lifeline: false,
                why,
                proof: "hook-replay",
            },
        );
    }

    // AN3 — measured class-1 rows reached through a launch idiom
    for (rel, hook, _stage5, why) in MEASURED_HOOK_DEPS {
        if !rels.contains(*rel) {
            errs.push(format!(
                "MEASURED_HOOK_DEPS: `{rel}` is not a file of the pack (RULINGS AN3)"
            ));
            continue;
        }
        if rows.get(*hook).map(|r| r.category) != Some("hook") {
            errs.push(format!(
                "MEASURED_HOOK_DEPS: `{rel}` names `{hook}`, which is not a registered hook (RULINGS AN3)"
            ));
            continue;
        }
        if rows.contains_key(*rel) {
            continue;
        }
        let text = (src.pack_file)(rel).map(|t| lf(&t));
        rows.insert(
            rel.to_string(),
            GenEntry {
                check: file_kind_check(rel, text.as_deref()),
                rel: rel.to_string(),
                category: "hook-dep",
                check_arg: String::new(),
                timeout_s: None,
                lifeline: false,
                why: format!("used by {hook}: {why}"),
                proof: "hook-replay",
            },
        );
    }

    // RULINGS BR1 — listed class-1 rows proven at stage 5: each must be a class-1 row
    for (rel, why) in STAGE5_HOOK_ROWS {
        match rows.get_mut(*rel) {
            Some(r) if r.category == "hook" || r.category == "hook-dep" => {
                r.why = format!("{} · stage-5 (RULINGS BR1): {why}", r.why);
            }
            _ => errs.push(format!(
                "STAGE5_HOOK_ROWS: `{rel}` is not a class-1 row (RULINGS BR1)"
            )),
        }
    }

    // RULINGS AT5 — the protection closure has NO drops: every file a protection row sources,
    //   launches or imports, transitively, is a protection row or a boot-critical row (so it can only
    //   change through `submit`, where the protection screen runs) — a measured drop does not count
    let on_a_list: BTreeSet<String> = rows
        .keys()
        .cloned()
        .chain(protection.iter().map(|e| e.rel.clone()))
        .collect();
    for root in protection.iter().filter(|e| rels.contains(e.rel.as_str())) {
        for (c, how) in protection_closure(src, &rels, &root.rel) {
            if !on_a_list.contains(&c) {
                errs.push(format!(
                    "protection closure: `{c}` ({how}; reached from the protection row `{}`) is on neither compiled list — list it; a measured drop does not count here (RULINGS AT5)",
                    root.rel
                ));
            }
        }
    }

    // AL3 · AN6 — the extended rule (imports wherever written, tools launched or opened by path,
    //   heredocs, the launch idioms of AN3) FINDS candidates, starting from the LISTED files and
    //   stopping at dropped files (one step: a new import inside an ordinary tool never fails the
    //   build); a candidate that is neither listed (either list) nor a measured drop is a NEW
    //   start-path file → build failure naming it (measure it — test_boot_critical_start_replay /
    //   the hook replay — then list it or add it to DROPPED_CANDIDATES with both break modes)
    let dropped: BTreeSet<&str> = DROPPED_CANDIDATES
        .iter()
        .chain(BY_CONSTRUCTION_DROPS.iter())
        .map(|(r, _)| *r)
        .collect();
    let protected: BTreeSet<&str> = protection.iter().map(|e| e.rel.as_str()).collect();
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    // AP5 — the protection rows are census roots too (a file a protection device launches or
    //   imports is a protection row, listed, or a measured drop)
    let roots: Vec<String> = rows
        .keys()
        .cloned()
        .chain(protection.iter().map(|e| e.rel.clone()))
        .collect();
    for r in &roots {
        let text = (src.pack_file)(r).map(|t| lf(&t)).unwrap_or_default();
        for (rel, how) in bindings(r, &text, &rels) {
            found.entry(rel).or_insert(how);
        }
    }
    for (rel, how) in &found {
        if rows.contains_key(rel)
            || protected.contains(rel.as_str())
            || dropped.contains(rel.as_str())
        {
            continue;
        }
        let base = rel.rsplit('/').next().unwrap_or(rel).to_string();
        if excluded.contains(&base) || is_clear_procedure_rel(rel) {
            continue;
        }
        errs.push(format!(
            "census: new start-path candidate `{rel}` ({how}) — neither listed nor a measured drop; measure both break modes and classify it (RULINGS AL1/AL3/AN2)"
        ));
    }
    // RULINGS BM2 — every launch site of a by-construction drop is a discarding form
    let class1: Vec<String> = rows
        .values()
        .filter(|e| e.category == "hook" || e.category == "hook-dep")
        .map(|e| e.rel.clone())
        .collect();
    errs.extend(by_construction_census(src, &rels, &class1));
    for (rel, _) in DROPPED_CANDIDATES
        .iter()
        .chain(BY_CONSTRUCTION_DROPS.iter())
    {
        if rows.contains_key(*rel) {
            errs.push(format!(
                "`{rel}` is both listed and in DROPPED_CANDIDATES — remove one (RULINGS AL3)"
            ));
        }
        if !rels.contains(*rel) {
            errs.push(format!(
                "DROPPED_CANDIDATES: `{rel}` is no longer a file of the pack — remove or rename the entry (RULINGS AN6)"
            ));
        }
    }

    // 6. the clear-procedure exclusion is absolute (also for anchored rows)
    for (rel, row) in &rows {
        let base = rel.rsplit('/').next().unwrap_or(rel);
        if excluded.contains(base) || is_clear_procedure_rel(rel) {
            errs.push(format!(
                "`{rel}` ({}) is a clear-procedure / statusline file and must not be boot-critical (C4 4.3, F2)",
                row.category
            ));
        }
    }

    if !errs.is_empty() {
        return Err(errs.join("\n"));
    }
    let mut out: Vec<GenEntry> = rows.into_values().chain(protection).collect();
    out.sort_by(|a, b| {
        a.rel.cmp(&b.rel).then_with(|| {
            let pa = CATEGORY_ORDER.iter().position(|c| *c == a.category);
            let pb = CATEGORY_ORDER.iter().position(|c| *c == b.category);
            pa.cmp(&pb)
        })
    });
    Ok(out)
}

fn rust_category(c: &str) -> &'static str {
    match c {
        "hook" => "EntryCategory::Hook",
        "hook-dep" => "EntryCategory::HookDep",
        "start-script" => "EntryCategory::StartScript",
        "start-dep" => "EntryCategory::StartDep",
        "config" => "EntryCategory::Config",
        _ => "EntryCategory::Protection",
    }
}

fn rust_check(c: &str) -> &'static str {
    match c {
        "hook-replay" => "CheckKind::HookReplay",
        "sh-syntax" => "CheckKind::ShSyntax",
        "py-compile" => "CheckKind::PyCompile",
        "json-parse" => "CheckKind::JsonParse",
        "self-test" => "CheckKind::SelfTest",
        _ => "CheckKind::NonePrerehearsal",
    }
}

/// Rust source of `OUT_DIR/boot_critical.rs` (included by `src/repair/boot_critical.rs`).
pub fn render(entries: &[GenEntry], inputs: &[(String, Vec<String>)]) -> String {
    let mut s = String::from(
        "// @generated by build.rs from src/repair/boot_critical_derive.rs (T5a · C4 · RULINGS B6) — do not edit.\n",
    );
    // RULINGS AI6: two lists — the boot-critical files, and the SIBLING protection list (C4 4.10)
    for (name, doc, protection) in [
        (
            "GENERATED",
            "The compiled boot-critical list (hooks, their dependencies, start scripts, core config) — the only source (C4 4.1).",
            false,
        ),
        (
            "GENERATED_PROTECTION",
            "The sibling list of the files implementing the four kept protections (C4 4.10 / D:941).",
            true,
        ),
    ] {
        let rows: Vec<&GenEntry> = entries
            .iter()
            .filter(|e| (e.category == "protection") == protection)
            .collect();
        s.push_str(&format!("/// {doc}\nstatic {name}: &[Entry] = &[\n"));
        for e in &rows {
            let t = match e.timeout_s {
                Some(n) => format!("Some({n})"),
                None => "None".to_string(),
            };
            s.push_str(&format!(
                "    Entry {{ rel: {:?}, category: {}, check: {}, check_arg: {:?}, timeout_s: {t}, lifeline: {} }},\n",
                e.rel,
                rust_category(e.category),
                rust_check(e.check),
                e.check_arg,
                e.lifeline
            ));
        }
        s.push_str(&format!(
            "];\n/// Why each row of [`{name}`] exists (same order).\nstatic {name}_WHY: &[&str] = &[\n"
        ));
        for e in &rows {
            s.push_str(&format!("    {:?},\n", e.why));
        }
        s.push_str("];\n");
        if !protection {
            // RULINGS AL1 — what proves each row: hook-replay | bootstrap-replay | real-level
            s.push_str(&format!(
                "/// What proves each row of [`{name}`] (same order): `hook-replay` (class 1), `bootstrap-replay` (class 2), `real-level` (class 3).\nstatic {name}_PROOF: &[&str] = &[\n"
            ));
            for e in &rows {
                s.push_str(&format!("    {:?},\n", e.proof));
            }
            s.push_str("];\n");
        }
    }
    // RULINGS AU1 — per protection test, the files it reads (sorted; every file on a list)
    s.push_str(
        "/// Per protection TEST (C · G · I3 · R), the files it reads — its rows and their protection closure (RULINGS AU1).\nstatic GENERATED_PROTECTION_INPUTS: &[(&str, &[&str])] = &[\n",
    );
    for (test, files) in inputs {
        let list: Vec<String> = files.iter().map(|f| format!("{f:?}")).collect();
        s.push_str(&format!("    ({test:?}, &[{}]),\n", list.join(", ")));
    }
    s.push_str("];\n");
    s
}
