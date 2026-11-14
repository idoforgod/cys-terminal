//! The protection screen's TABLES (CONTRACTS C4 — RULINGS AY2, BA2, BA5, BA6, BC3, BC4), stated
//! once: the deny classes of the released devices (corpora G, R, C), their probes, the cell table
//! E-1 … E-21, the I4 region anchors and the census anchors. Track T5b.
//!
//! Placeholders inside a probe's input (replaced per run, JSON-safe): `{pack}` the pack
//! `CYS_PACK_DIR` names in the run (`<scratch HOME>/.cys/pack` in E-9), `{w}` the run's working
//! folder ("<scratch>" of C4), `{home}` the scratch HOME, `{other}` a pack of ANOTHER lane that is
//! never the run's pack (`{home}/.cys/pack-dept-zz`).

/// The two hook devices the probes run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Device {
    /// `hooks/guard.sh` — corpus G (protection test `G`).
    Guard,
    /// `hooks/role-capability-gate.sh` — corpora R and C (protection tests `R`, `C`).
    Gate,
}

impl Device {
    pub fn rel(self) -> &'static str {
        match self {
            Device::Guard => "hooks/guard.sh",
            Device::Gate => "hooks/role-capability-gate.sh",
        }
    }
}

/// One probe: a hook input built from a tool call (or a raw input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// The tool name; empty for a raw input.
    pub tool: &'static str,
    /// The tool input as JSON (placeholders inside strings) — or, for a raw probe, the raw hook
    /// input text itself.
    pub input: &'static str,
    /// The input is sent as it is (not wrapped into a hook input) — G-24 / R-16's parser arm.
    pub raw: bool,
    /// Pad the `content` field of the tool input to this many bytes (C-9, C-25).
    pub pad: usize,
    /// Seed the session's tool-call counter at the budget (C-23).
    pub seed_budget: bool,
}

const fn tool(tool: &'static str, input: &'static str) -> Probe {
    Probe {
        tool,
        input,
        raw: false,
        pad: 0,
        seed_budget: false,
    }
}

const fn bash(_cmd_json: &'static str) -> Probe {
    tool("Bash", _cmd_json)
}

const RAW_UNPARSEABLE: Probe = Probe {
    tool: "",
    input: "{",
    raw: true,
    pad: 0,
    seed_budget: false,
};

/// One deny class of a released device (C4 AY2 tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Class {
    pub id: &'static str,
    pub test: &'static str,
    pub device: Device,
    /// The role of the seat the probe stands for (`CYS_ROLE`, the stub's `surface-role` answer).
    pub role: &'static str,
    /// At least one probe; every probe runs in every cell its device reads (near-variants extra).
    pub probes: &'static [Probe],
    /// The released device line(s) the class comes from (for the report).
    pub line: &'static str,
}

const GW: &str = "worker"; // guard.sh reads no role; the seat is an ordinary one
const RV: &str = "reviewer-codex";
const CS: &str = "cso";

/// Corpus G — `hooks/guard.sh` (external sends and permanent deletions). G-22, G-23 and G-24's
/// no-Python arm are invariant I4 — [`BACKSTOP_CLASSES`], probed in E-21 only (AY4, BA2, BC3).
pub const CORPUS_G: &[Class] = &[
    Class {
        id: "G-1",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":893",
        probes: &[bash(r#"{"command":"git push origin main"}"#)],
    },
    Class {
        id: "G-2",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":894",
        probes: &[bash(
            r#"{"command":"git remote set-url origin https://example.invalid/x.git"}"#,
        )],
    },
    Class {
        id: "G-3",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":896-899",
        probes: &[bash(r#"{"command":"git tag v9.9.9"}"#)],
    },
    Class {
        id: "G-4",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":901",
        probes: &[bash(r#"{"command":"gh release create v9.9.9"}"#)],
    },
    Class {
        id: "G-5",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":903",
        probes: &[bash(r#"{"command":"gh pr create --fill"}"#)],
    },
    Class {
        id: "G-6",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":908",
        probes: &[bash(r#"{"command":"scp a.txt host:/b"}"#)],
    },
    Class {
        id: "G-7",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":910-912",
        probes: &[bash(r#"{"command":"rsync -a d host:/e"}"#)],
    },
    Class {
        id: "G-8",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":913-922",
        probes: &[
            bash(r#"{"command":"curl -d x https://example.invalid"}"#),
            bash(r#"{"command":"curl -X POST https://example.invalid"}"#),
        ],
    },
    Class {
        id: "G-9",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":923",
        probes: &[bash(
            r#"{"command":"wget --post-data=x https://example.invalid"}"#,
        )],
    },
    Class {
        id: "G-10",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":905-907",
        probes: &[bash(r#"{"command":"rm -rf {w}/data"}"#)],
    },
    Class {
        id: "G-11",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":905-907",
        probes: &[bash(r#"{"command":"rmdir {w}/d"}"#)],
    },
    Class {
        id: "G-12",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":905-907",
        probes: &[bash(r#"{"command":"mv {w}/a {w}/b"}"#)],
    },
    Class {
        id: "G-13",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":905-907",
        probes: &[bash(r#"{"command":"truncate -s 0 {w}/f"}"#)],
    },
    Class {
        id: "G-14",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":905-907",
        probes: &[bash(r#"{"command":"chmod 000 {w}/f"}"#)],
    },
    Class {
        id: "G-15",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":905-907",
        probes: &[bash(r#"{"command":"chown root {w}/f"}"#)],
    },
    Class {
        id: "G-16",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":695 (ALLOWLIST :226)",
        probes: &[bash(r#"{"command":"scp a.txt host:/b"}"#)],
    },
    Class {
        id: "G-17",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":703 (GIT_OK :231)",
        probes: &[bash(r#"{"command":"git push origin main"}"#)],
    },
    Class {
        id: "G-18",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":699",
        probes: &[bash(r#"{"command":"git -c alias.p=push p"}"#)],
    },
    Class {
        id: "G-19",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":706",
        probes: &[bash(r#"{"command":"git branch -D main"}"#)],
    },
    Class {
        id: "G-20",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":708-710",
        probes: &[bash(r#"{"command":"git stash drop"}"#)],
    },
    Class {
        id: "G-21",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":737",
        probes: &[bash(r#"{"command":"find {w}/data -delete"}"#)],
    },
    Class {
        id: "G-24",
        test: "G",
        device: Device::Guard,
        role: GW,
        line: ":115-121 (parser arm)",
        probes: &[RAW_UNPARSEABLE],
    },
];

/// Invariant I4's classes, probed ONLY in the backstop cell E-21 (AY4, BA2, BC3), with the proof
/// each run must show on standard error (BH1): (class, probe, autopilot flag, deny reason line).
pub const BACKSTOP_CLASSES: &[(&str, Probe, bool, &str)] = &[
    (
        "G-22",
        bash(r#"{"command":"git push origin main"}"#),
        false,
        "AUTOPILOT GUARD DENY: 백스톱: 비가역 명령. 오너 승인 필요",
    ),
    (
        "G-23",
        bash(r#"{"command":"rm -rf {w}/data"}"#),
        false,
        "AUTOPILOT GUARD DENY: 백스톱: 파괴적 파일연산. 오너 승인 필요",
    ),
    (
        "G-24np",
        bash(r#"{"command":"curl -d x https://example.invalid"}"#),
        true,
        "AUTOPILOT GUARD DENY: [STRICT] python3 미가용 — parser 부재 fail-closed deny(자율주행)",
    ),
];

/// The backstop's own marker line (guard.sh `run_loose_backstop`, printed first — BH1).
pub const BACKSTOP_MARKER: &str =
    "AUTOPILOT GUARD: python3 미가용 → bash 백스톱(LOOSE degraded) 모드";

/// Corpus R — reviewer writes the capability gate must deny.
pub const CORPUS_R: &[Class] = &[
    Class {
        id: "R-1",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "decide :2980-2986 · path_is_allowed",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{pack}/bin/x.py","content":"x"}"#,
        )],
    },
    Class {
        id: "R-2",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "path_is_allowed (ledger)",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{home}/.cys/local/repairs/R-1/record.json","content":"x"}"#,
        )],
    },
    Class {
        id: "R-3",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "reviewer_exact_allowed",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{other}/round/_reviews/v.json","content":"x"}"#,
        )],
    },
    Class {
        id: "R-4",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "path_is_allowed (repository)",
        probes: &[tool(
            "Edit",
            r#"{"file_path":"/Users/x/dev/repo/src/a.rs","old_string":"a","new_string":"b"}"#,
        )],
    },
    Class {
        id: "R-5",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "_is_gate_state_path",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{home}/.cys/state/capgate/cys-capgate-x","content":"x"}"#,
        )],
    },
    Class {
        id: "R-6",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "bash_write_reason (redirect)",
        probes: &[bash(r#"{"command":"echo x > {pack}/hooks/y.sh"}"#)],
    },
    Class {
        id: "R-7",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "WRITE_SHELL_CMDS",
        probes: &[bash(r#"{"command":"rm -rf {pack}/bin"}"#)],
    },
    Class {
        id: "R-8",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "GIT_WRITE_SUBS",
        probes: &[bash(r#"{"command":"git commit -m x"}"#)],
    },
    Class {
        id: "R-9",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "nested shell / eval",
        probes: &[bash(r#"{"command":"sh -c \"rm -rf {pack}/bin\""}"#)],
    },
    Class {
        id: "R-10",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "builder / package-tool write",
        probes: &[bash(r#"{"command":"npm install x"}"#)],
    },
    Class {
        id: "R-11",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "BUILDER_ENV_WRITE",
        probes: &[bash(r#"{"command":"RUSTC_WRAPPER=/tmp/x cargo check"}"#)],
    },
    Class {
        id: "R-12",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "glob in command position",
        probes: &[bash(r#"{"command":"/bin/r? -rf {pack}/bin"}"#)],
    },
    Class {
        id: "R-13",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "unreadable command (fail-closed)",
        probes: &[bash(r#"{"command":"git 'push"}"#)],
    },
    Class {
        id: "R-14",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "git_sub_is_write (GIT_FILE_OUT_OPTS)",
        probes: &[bash(r#"{"command":"git diff --output={pack}/bin/x.py"}"#)],
    },
    Class {
        id: "R-15",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "_git_config_is_write",
        probes: &[bash(
            r#"{"command":"git config --add core.hooksPath /tmp/x"}"#,
        )],
    },
    Class {
        id: "R-16",
        test: "R",
        device: Device::Gate,
        role: RV,
        line: "_undecidable · no-Python :349-354 · oversize :338-342",
        probes: &[RAW_UNPARSEABLE],
    },
];

/// Corpus C — CSO writes, bash verdicts and repair mutations the capability gate must deny.
pub const CORPUS_C: &[Class] = &[
    Class {
        id: "C-1",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "CSO_DENY_TOOLS",
        probes: &[tool("Agent", r#"{"prompt":"x","description":"x"}"#)],
    },
    Class {
        id: "C-2",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "computer-use MCP",
        probes: &[tool("mcp__computer-use__screenshot", r#"{}"#)],
    },
    Class {
        id: "C-3",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "Skill allow list",
        probes: &[tool("Skill", r#"{"skill":"brainstorming"}"#)],
    },
    Class {
        id: "C-4",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "cso_path_allowed (roots)",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{pack}/bin/x.py","content":"x"}"#,
        )],
    },
    Class {
        id: "C-5",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "cso_path_allowed (another lane)",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{other}/round/CSO_TODO.md","content":"x"}"#,
        )],
    },
    Class {
        id: "C-6",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "own round/, not CSO-owned",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{pack}/round/MASTER_TODO.md","content":"x"}"#,
        )],
    },
    Class {
        id: "C-7",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "_is_protected_state_path",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{home}/.cys/state/mission.json","content":"x"}"#,
        )],
    },
    Class {
        id: "C-8",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "gate state",
        probes: &[tool(
            "Write",
            r#"{"file_path":"{home}/.cys/state/capgate/cys-capgate-x","content":"x"}"#,
        )],
    },
    Class {
        id: "C-9",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "todo_cap_verdict (64 KB)",
        probes: &[Probe {
            tool: "Write",
            input: r#"{"file_path":"{pack}/round/CSO_TODO.md","content":""}"#,
            raw: false,
            pad: 70 * 1024,
            seed_budget: false,
        }],
    },
    Class {
        id: "C-10",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "cso_bash_verdict (background)",
        probes: &[
            bash(r#"{"command":"sleep 9","run_in_background":true}"#),
            bash(r#"{"command":"sleep 9 &"}"#),
        ],
    },
    Class {
        id: "C-11",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "redirect outside roots",
        probes: &[bash(r#"{"command":"echo x > {pack}/hooks/y.sh"}"#)],
    },
    Class {
        id: "C-12",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "context env / exec by path",
        probes: &[
            bash(r#"{"command":"CYS_PACK_DIR=/x cys status"}"#),
            bash(r#"{"command":"/bin/rm x"}"#),
        ],
    },
    Class {
        id: "C-13",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "_ro_segment_verdict",
        probes: &[bash(r#"{"command":"rm -rf {pack}/state"}"#)],
    },
    Class {
        id: "C-14",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "git non-read",
        probes: &[bash(r#"{"command":"git commit -m x"}"#)],
    },
    Class {
        id: "C-15",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "_py_segment_verdict",
        probes: &[bash(r#"{"command":"python3 -c \"print(1)\""}"#)],
    },
    Class {
        id: "C-16",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "_cys_segment_verdict (CSO_CYS_SUBVERBS)",
        probes: &[bash(
            r#"{"command":"cys repair begin --files bin/x.py --trigger grant"}"#,
        )],
    },
    Class {
        id: "C-17",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "cys verb allow lists",
        probes: &[bash(r#"{"command":"cys approval sign x"}"#)],
    },
    Class {
        id: "C-18",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "TTL verbs",
        probes: &[bash(r#"{"command":"cys kill 7"}"#)],
    },
    Class {
        id: "C-19",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "daemon-changing option",
        probes: &[bash(r#"{"command":"cys --socket /other.sock status"}"#)],
    },
    Class {
        id: "C-20",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "effect-changing option",
        probes: &[bash(
            r#"{"command":"cys send --clear-first --to master x"}"#,
        )],
    },
    Class {
        id: "C-21",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "cys send not --queued --to master",
        probes: &[bash(r#"{"command":"cys send --to worker x"}"#)],
    },
    Class {
        id: "C-22",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "endless stream",
        probes: &[
            bash(r#"{"command":"cys events"}"#),
            bash(r#"{"command":"tail -f {pack}/round/x.log"}"#),
        ],
    },
    Class {
        id: "C-23",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "BUDGET_DENY (tool_calls ≥ 2,000)",
        probes: &[Probe {
            tool: "Read",
            input: r#"{"file_path":"{pack}/bin/x.py"}"#,
            raw: false,
            pad: 0,
            seed_budget: true,
        }],
    },
    Class {
        id: "C-24",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "cso_split (fail-closed)",
        probes: &[bash(r#"{"command":"cat $(echo x)"}"#)],
    },
    Class {
        id: "C-25",
        test: "C",
        device: Device::Gate,
        role: CS,
        line: "oversize input without a temp file :343-346",
        probes: &[Probe {
            tool: "Write",
            input: r#"{"file_path":"{pack}/round/CSO_NOTES.md","content":""}"#,
            raw: false,
            pad: 300 * 1024,
            seed_budget: false,
        }],
    },
];

/// Every probed class, G then R then C.
pub fn all_classes() -> impl Iterator<Item = &'static Class> {
    CORPUS_G
        .iter()
        .chain(CORPUS_R.iter())
        .chain(CORPUS_C.iter())
}

/// How a cell is set up (C4 cell table; baseline BA5 = E-1 + E-4 + E-8 + E-12 + E-16 + the bundled
/// interpreter — cells whose set-up IS the baseline share one run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Setup {
    Baseline,
    /// E-2: `CYS_SOCKET` names a path holding no socket.
    NoSocket,
    /// E-3: socket present, PATH without the stub folder, `CYS_BIN` absent.
    NoCys,
    /// E-5: `AUTOPILOT_ACTIVE` in the deciding hook's pack.
    Active,
    /// E-6: `AUTOPILOT_PAUSED`.
    Paused,
    /// E-7: both flags.
    ActivePaused,
    /// E-9: the variant at `<HOME>/.cys/pack`, `CYS_PACK_DIR` absent.
    PackAtHome,
    /// E-10: the hook runs from the variant, `CYS_PACK_DIR` = another lane's pack (RELEASED inputs).
    OtherLane,
    /// E-11: the hook runs from another lane's pack (RELEASED), `CYS_PACK_DIR` = the variant.
    FromOtherLane,
    /// E-13: a repeated call inside the cache time; the second is judged.
    Warm,
    /// E-14: answered query, socket removed, a call < 15 s later and one < 5 s after it (both judged).
    CacheFallback,
    /// E-15: as E-14 with `CYS_ROLE` = `worker`.
    CacheEnvDisagree,
    /// E-17: `TMPDIR` not writable.
    TmpReadonly,
    /// E-18: no interpreter for the gate (`CYS_PY` absent, no python on PATH).
    NoInterpreter,
    /// E-19: E-10 + `AUTOPILOT_ACTIVE` in the pack `CYS_PACK_DIR` names.
    ActiveOtherLane,
    /// E-20: E-11 + `AUTOPILOT_ACTIVE` in the variant.
    ActiveFromOtherLane,
    /// E-21: the backstop cell (special — [`BACKSTOP_CLASSES`]).
    Backstop,
}

/// One cell of the cell table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub id: &'static str,
    pub dimension: &'static str,
    pub devices: &'static [Device],
    pub setup: Setup,
}

const GG: &[Device] = &[Device::Guard, Device::Gate];
const G_: &[Device] = &[Device::Guard];
const _G: &[Device] = &[Device::Gate];

/// THE CELL TABLE (C4 AY2): 21 cells — guard.sh reads 16 (S, A, P, T, E-19, E-20, E-21), the
/// capability gate 14 (S, P, C, T, N).
pub const CELLS: &[Cell] = &[
    Cell {
        id: "E-1",
        dimension: "S",
        devices: GG,
        setup: Setup::Baseline,
    },
    Cell {
        id: "E-2",
        dimension: "S",
        devices: GG,
        setup: Setup::NoSocket,
    },
    Cell {
        id: "E-3",
        dimension: "S",
        devices: GG,
        setup: Setup::NoCys,
    },
    Cell {
        id: "E-4",
        dimension: "A",
        devices: G_,
        setup: Setup::Baseline,
    },
    Cell {
        id: "E-5",
        dimension: "A",
        devices: G_,
        setup: Setup::Active,
    },
    Cell {
        id: "E-6",
        dimension: "A",
        devices: G_,
        setup: Setup::Paused,
    },
    Cell {
        id: "E-7",
        dimension: "A",
        devices: G_,
        setup: Setup::ActivePaused,
    },
    Cell {
        id: "E-8",
        dimension: "P",
        devices: GG,
        setup: Setup::Baseline,
    },
    Cell {
        id: "E-9",
        dimension: "P",
        devices: GG,
        setup: Setup::PackAtHome,
    },
    Cell {
        id: "E-10",
        dimension: "P",
        devices: GG,
        setup: Setup::OtherLane,
    },
    Cell {
        id: "E-11",
        dimension: "P",
        devices: GG,
        setup: Setup::FromOtherLane,
    },
    Cell {
        id: "E-12",
        dimension: "C",
        devices: _G,
        setup: Setup::Baseline,
    },
    Cell {
        id: "E-13",
        dimension: "C",
        devices: _G,
        setup: Setup::Warm,
    },
    Cell {
        id: "E-14",
        dimension: "C",
        devices: _G,
        setup: Setup::CacheFallback,
    },
    Cell {
        id: "E-15",
        dimension: "C",
        devices: _G,
        setup: Setup::CacheEnvDisagree,
    },
    Cell {
        id: "E-16",
        dimension: "T",
        devices: GG,
        setup: Setup::Baseline,
    },
    Cell {
        id: "E-17",
        dimension: "T",
        devices: GG,
        setup: Setup::TmpReadonly,
    },
    Cell {
        id: "E-18",
        dimension: "N",
        devices: _G,
        setup: Setup::NoInterpreter,
    },
    Cell {
        id: "E-19",
        dimension: "AxP",
        devices: G_,
        setup: Setup::ActiveOtherLane,
    },
    Cell {
        id: "E-20",
        dimension: "AxP",
        devices: G_,
        setup: Setup::ActiveFromOtherLane,
    },
    Cell {
        id: "E-21",
        dimension: "backstop",
        devices: G_,
        setup: Setup::Backstop,
    },
];

/// The cells a device reads, in table order.
pub fn cells_of(d: Device) -> impl Iterator<Item = &'static Cell> {
    CELLS.iter().filter(move |c| c.devices.contains(&d))
}

// ───────────────────────────── invariant I4 — the two regions ─────────────────────────────

/// Invariant I4's regions of `hooks/guard.sh` (AY4, BA4, BD5), named by anchors (no anchor
/// comment is added to the device): (name, first line, last line) — the first line that equals
/// `first` (trimmed of trailing white space) up to and including the first following line that
/// equals `last`. At base 36b00699: interpreter resolution :67–:85, `run_loose_backstop` :1045–:1062.
pub const I4_REGIONS: &[(&str, &str, &str)] = &[
    (
        "interpreter-resolution",
        "PYBIN=\"\"",
        "[ \"${GUARD_TEST_MODE:-0}\" = \"1\" ] && [ \"${GUARD_FORCE_NOPY:-0}\" = \"1\" ] && PYBIN=\"\"",
    ),
    ("bash-backstop", "run_loose_backstop() {", "}"),
];

/// The text of an I4 region in `text`, or `None` when an anchor cannot be found.
pub fn i4_region<'a>(text: &'a str, first: &str, last: &str) -> Option<String> {
    let lines: Vec<&'a str> = text.split('\n').collect();
    let start = lines.iter().position(|l| l.trim_end() == first)?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_end() == last)
        .map(|i| i + start + 1)?;
    Some(lines[start..=end].join("\n"))
}

// ───────────────────────────── census (iii) — deny rules and branches ─────────────────────────────

/// The text an anchor of [`CENSUS_PINS`] is compared with: the line without quote, bracket and
/// backslash characters, white space collapsed to single spaces and trimmed — so the pin table holds
/// plain strings (no escape, no bracket inside the block — RULINGS CO1 / CS1).
pub fn anchor_text(line: &str) -> String {
    let kept: String = line
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '[' | ']' | '\\'))
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// RULINGS CZ7 — one census region of a device text: from the ONE line whose [`anchor_text`] is
/// `start` up to, not including, the ONE line whose anchor text is `end` (`""` = to the end of the
/// text). Each anchor must occur exactly once in the whole text, the end after the start. Returns
/// (line count, sha256 hex of the region's lines joined by LF) — the region pinned WHOLE.
pub fn census_region(text: &str, start: &str, end: &str) -> Result<(usize, String), String> {
    use sha2::{Digest, Sha256};
    let lines: Vec<&str> = text.split('\n').collect();
    let at = |a: &str| -> Result<usize, String> {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| anchor_text(l) == a)
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("anchor not found: {a:?}")),
            _ => Err(format!("anchor occurs {} times: {a:?}", hits.len())),
        }
    };
    let s = at(start)?;
    let e = if end.is_empty() { lines.len() } else { at(end)? };
    if e <= s {
        return Err(format!("anchor {end:?} does not follow {start:?}"));
    }
    let region = lines[s..e].join("\n");
    let h = Sha256::digest(region.as_bytes());
    Ok((e - s, h.iter().map(|b| format!("{b:02x}")).collect()))
}

/// RULINGS CZ7 — census (iii): each released device's DENY-DECISION region and ENVIRONMENT-BRANCH
/// region(s), pinned WHOLE between explicit anchors ([`census_region`]): (device, region, start
/// anchor, end anchor — `""` = to the end, line count, sha256 of the region). guard.sh: its bash head
/// (lane redirect, script folder, the autopilot flag files of the test mode, the interpreter
/// resolution, the deny output, the flag trust check, the python environment reads and input parse,
/// the normalisers), its python decision code (protected names, wrappers, STRICT allow-lists and
/// `git`/`cys` gating, the LOOSE rules — the `gh pr create/merge` condition among them — and the
/// dispatch), and its bash tail (constitution token, bash backstop, dispatch). The gate: its bash
/// head (lane redirect, role cache, gated-role predicate, role resolution, self-test switch, the
/// big-input and no-interpreter branches) and its python decision code up to the self-tests (every
/// table — WRITE_SHELL_CMDS, GIT_WRITE_SUBS, CSO_CYS_SUBVERBS, BUILDER_ENV_WRITE … — and `main`).
/// `_lib.sh`: everything after its header (interpreter resolution and shim checks, lane redirect,
/// role helpers). Headers, comments outside the regions and the gate's self-tests are not pinned. A
/// change inside a region fails `screen::tests::census_iii_deny_rules_and_branches_pinned` until the
/// corpora and cells are reviewed against the device and this table is updated with them (CM4: a
/// lib-suite test).
pub const CENSUS_PINS: &[(&str, &str, &str, &str, usize, &str)] = &[
    (
        "hooks/guard.sh",
        "environment-head",
        "command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect $@",
        "def _strip_hidden(s):",
        90,
        "df69e734af1511381e3ab5c7e560d348a7b30cfb76effaa517f463be739ff3ec",
    ),
    (
        "hooks/guard.sh",
        "deny-decision",
        "def _strip_hidden(s):",
        "PY",
        897,
        "51e6619b1db565200afdd6dcf4bc8f1d6915f6cd88d089fb127726d709ef8b81",
    ),
    (
        "hooks/guard.sh",
        "backstop-and-dispatch",
        "PY",
        "",
        69,
        "f61562abaffeddd6a51f08ac239b6d5b1d6cf1b3c6950f7328374b9dc48f24f5",
    ),
    (
        "hooks/role-capability-gate.sh",
        "environment-head",
        "command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect $@",
        "exec $CYS_PY - <<PYEOF",
        276,
        "3b869a6f67b58b414efa99fa5c9350f5a331b8f357d4b1dcaf16f649c73edb39",
    ),
    (
        "hooks/role-capability-gate.sh",
        "deny-decision",
        "exec $CYS_PY - <<PYEOF",
        "def self_test():",
        3095,
        "bbaa170bc98b3f380936aece6cdde45e365a635ea0186ce07eaa39cd3115e8cb",
    ),
    (
        "hooks/_lib.sh",
        "environment",
        "-n ${CYS_LIB_SH_LOADED:-} && return 0",
        "",
        1063,
        "e03702093e8d982f160ccb39c495851614e0e439e219cb1c862351d0e79f4413",
    ),
];
