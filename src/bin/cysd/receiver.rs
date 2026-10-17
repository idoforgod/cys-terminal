// Wired into the writer and producers by the following S-a commits; until then only tests use it.
#![allow(dead_code)]
//! S-a receiver policy, discovery and binding (DESIGN-S-FROZEN-R3 §4.1-4.2, HEAD-R3-RULING S4-2/S4-4).
//!
//! A `ReceiverPolicy` is fixed BEFORE any binding attempt from the registered adapter declaration;
//! a failed or denied full proof never downgrades it (S3-4). A `Binding` is the daemon's own native
//! observation of the seat root and of the one agent invocation that owns the terminal input. The
//! writer re-checks it before every OS call. Nothing here writes to a PTY.
//!
//! What this module cannot establish (named residuals, never claimed): stdin ownership itself,
//! immutable launch history (argv is mutable), and an atomic check-then-write; a process can die
//! after a passing check. Those are S-b containment topics.

use cys::process_observation::{Failure, ProcessObservation, Retained};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Built-in adapters with a shipped invocation contract (agents.json `_schema: 3` keys).
pub const BUILTIN_ADAPTERS: [&str; 4] = ["claude", "codex", "gemini", "grok"];

/// Per-attempt caller deadline. PROVISIONAL until the native discovery-cost cells (S-AC15) are
/// measured on macOS and Windows (BUILD_PLAN rule 8: no release while a Windows budget is provisional).
pub const DISCOVERY_CALLER_DEADLINE_PROVISIONAL_MS: u64 = 250;
/// At most four started native discoveries per proof obligation (design 4.1/4.6).
pub const MAX_STARTS_PER_OBLIGATION: u32 = 4;
/// Minimum spacing between two started discoveries of one obligation.
pub const MIN_START_SPACING_MS: u64 = 250;
/// Bounded executor size (design: at least two independently scheduled workers).
pub const DISCOVERY_WORKERS: usize = 2;
/// Boot proof window after eligibility (design 4.6, fixed).
pub const BOOT_PROOF_WINDOW_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofClass {
    FullRequired,
    ObservedAgentWeak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationOrigin {
    LaunchAgent,
    ClaimRole,
    Reclaim,
    HandObserved,
    Restored,
    DeclaredCustom,
}

impl RegistrationOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LaunchAgent => "launch-agent",
            Self::ClaimRole => "claim-role",
            Self::Reclaim => "reclaim",
            Self::HandObserved => "hand-observed",
            Self::Restored => "restored",
            Self::DeclaredCustom => "declared-custom",
        }
    }
}

/// The named residual risk carried by every weak receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeakRisk {
    /// Built-in adapter id whose configured command is not the shipped declaration.
    CustomWrapperReceiverIdentity,
    /// Adapter id outside the built-ins.
    DeclaredCustomInvocation,
    /// The OS has no native observation backend (not macOS/Windows/Linux).
    NoNativeBackend,
}

impl WeakRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CustomWrapperReceiverIdentity => "custom_wrapper_receiver_identity",
            Self::DeclaredCustomInvocation => "unregistered_receiver_identity",
            Self::NoNativeBackend => "no_native_backend",
        }
    }
}

/// Which process slot carries the adapter: its own executable, or a script run by a runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipe {
    /// Base name of the configured binary (`agent_candidates_from_json` semantics).
    pub bin: String,
    /// A declaration-derived constraint, never inferred from a matching process.
    #[serde(default)]
    pub declared_invocation: Option<DeclaredInvocation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredInvocation {
    pub argv: Vec<Vec<u8>>,
    pub script_index: Option<usize>,
    pub allowed_suffixes: Vec<Vec<DeclaredArgument>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclaredArgument {
    Literal(Vec<u8>),
    SessionId { prefix: Vec<u8>, suffix: Vec<u8> },
}

impl DeclaredArgument {
    fn matches(&self, actual: &[u8]) -> bool {
        match self {
            Self::Literal(value) => value == actual,
            Self::SessionId { prefix, suffix } => actual.len() > prefix.len() + suffix.len()
                && actual.starts_with(prefix) && actual.ends_with(suffix),
        }
    }
}

// Raw serialization is explicit private persistence, not the public receipt shape.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct ReceiverPolicy {
    pub policy_epoch: u64,
    /// Exact frozen declaration and its digest, never derived from a successful native match.
    pub declaration: String,
    pub config_digest: String,
    pub adapter: String,
    pub recipe: Recipe,
    pub origin: RegistrationOrigin,
    pub class: ProofClass,
    pub weak_risk: Option<WeakRisk>,
}

/// Public event/status/receipt projection: no command, executable recipe or environment.
#[derive(Clone, Debug, Serialize)]
pub struct ReceiverPolicySummary<'a> {
    pub policy_epoch: u64,
    pub adapter: &'a str,
    pub config_digest: &'a str,
    pub origin: RegistrationOrigin,
    pub class: ProofClass,
    pub weak_risk: Option<WeakRisk>,
}

/// Explicit raw persistence view. No Debug and no public-field construction. Serialize this
/// only to the private policy store; event/status callers use public_summary instead.
#[derive(Serialize)]
pub(crate) struct PrivateReceiverPolicy<'a> {
    policy_epoch: u64,
    declaration: &'a str,
    config_digest: &'a str,
    adapter: &'a str,
    recipe: &'a Recipe,
    origin: RegistrationOrigin,
    class: ProofClass,
    weak_risk: Option<WeakRisk>,
}

impl ReceiverPolicy {
    pub fn public_summary(&self) -> ReceiverPolicySummary<'_> {
        ReceiverPolicySummary {
            policy_epoch: self.policy_epoch,
            adapter: &self.adapter,
            config_digest: &self.config_digest,
            origin: self.origin,
            class: self.class,
            weak_risk: self.weak_risk,
        }
    }

    pub(crate) fn for_private_persistence(&self) -> PrivateReceiverPolicy<'_> {
        PrivateReceiverPolicy {
            policy_epoch: self.policy_epoch,
            declaration: &self.declaration,
            config_digest: &self.config_digest,
            adapter: &self.adapter,
            recipe: &self.recipe,
            origin: self.origin,
            class: self.class,
            weak_risk: self.weak_risk,
        }
    }
}

impl std::fmt::Debug for ReceiverPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiverPolicy")
            .field("summary", &self.public_summary())
            .finish_non_exhaustive()
    }
}

/// Fix the proof class from the declaration alone (never from a discovery result).
/// `shipped_bin` is the embedded (shipped) declaration's bin for that adapter, if any.
pub fn classify_policy(
    adapter: &str,
    configured_bin: &str,
    shipped_bin: Option<&str>,
    native_backend: bool,
) -> (ProofClass, Option<WeakRisk>) {
    if !native_backend {
        return (
            ProofClass::ObservedAgentWeak,
            Some(WeakRisk::NoNativeBackend),
        );
    }
    let builtin = BUILTIN_ADAPTERS.contains(&adapter);
    match (builtin, shipped_bin) {
        (true, Some(shipped)) if bin_eq(shipped, configured_bin) => {
            (ProofClass::FullRequired, None)
        }
        (true, _) => (
            ProofClass::ObservedAgentWeak,
            Some(WeakRisk::CustomWrapperReceiverIdentity),
        ),
        (false, _) => (
            ProofClass::ObservedAgentWeak,
            Some(WeakRisk::DeclaredCustomInvocation),
        ),
    }
}

/// True on the platforms that have a native observation backend.
pub const fn native_backend_available() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux", windows))
}

fn strip_exe(name: &str) -> (&str, bool) {
    let lower_ends = |suf: &str| {
        name.len() > suf.len()
            && name
                .get(name.len() - suf.len()..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(suf))
    };
    for suf in [".exe", ".cmd", ".bat", ".ps1"] {
        if lower_ends(suf) {
            return (&name[..name.len() - suf.len()], true);
        }
    }
    (name, false)
}

fn bin_eq(a: &str, b: &str) -> bool {
    let (a, wa) = strip_exe(base_name(a));
    let (b, wb) = strip_exe(base_name(b));
    if wa || wb || cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Where the adapter sits in one process's invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The process's own executable (or argv0 inside the adapter's versioned install dir).
    Native,
    /// `runtime [runtime options] <script> args…`; the index is the script position in argv.
    Script(usize),
}

const SCRIPT_RUNTIMES: [&str; 3] = ["node", "nodejs", "bun"];

/// Strict slot match: only the executable, argv0-in-versioned-install-dir, or the runtime's script
/// position counts — never an arbitrary argument (a `tail` operand or help text naming the agent).
pub fn match_slot(recipe: &Recipe, obs: &ProcessObservation) -> Option<Slot> {
    obs.invocation.argv.first().filter(|arg| !arg.is_empty())?;
    if let Some(declared) = &recipe.declared_invocation {
        let expected0 = std::str::from_utf8(declared.argv.first()?).ok()?;
        if !bin_eq(expected0, &recipe.bin) || declared.argv.first()?.is_empty() {
            return None;
        }
        let slot = if let Some(index) = declared.script_index {
            let exe = std::str::from_utf8(&obs.identity.executable).ok()?;
            if !is_script_runtime(&recipe.bin) || !same_script_runtime(exe, &recipe.bin)
                || runtime_script_slot(&declared.argv) != Some(index)
                || runtime_script_slot(&obs.invocation.argv) != Some(index)
            {
                return None;
            }
            Slot::Script(index)
        } else {
            if generic_interpreter_recipe(&recipe.bin) { return None; }
            match_unconstrained_slot(recipe, obs)?
        };
        // For a declared runtime, preserve runtime options AND the exact script argument.
        // For native/npm agent programs, compare arguments after their actual invocation slot.
        let actual = if declared.script_index.is_some() {
            obs.invocation.argv.get(1..)?
        } else {
            adapter_args(obs, slot)
        };
        let expected = declared.argv.get(1..)?;
        if !actual.starts_with(expected) { return None; }
        let suffix = &actual[expected.len()..];
        if !suffix.is_empty() && !declared.allowed_suffixes.iter().any(|pattern| {
            pattern.len() == suffix.len() && pattern.iter().zip(suffix).all(|(p, a)| p.matches(a))
        }) { return None; }
        return Some(slot);
    }
    // Old/private data and direct constructors cannot revive an interpreter-as-agent recipe.
    // This is a known-infrastructure guard; current custom declarations also retain their args.
    if generic_interpreter_recipe(&recipe.bin) { return None; }
    match_unconstrained_slot(recipe, obs)
}

pub(crate) fn is_script_runtime(bin: &str) -> bool {
    let (name, _) = strip_exe(base_name(bin));
    SCRIPT_RUNTIMES.iter().any(|runtime| name.eq_ignore_ascii_case(runtime))
}

fn same_script_runtime(a: &str, b: &str) -> bool {
    let normalize = |name: &str| {
        let (name, _) = strip_exe(base_name(name));
        match name.to_ascii_lowercase().as_str() {
            "node" | "nodejs" => Some("node"), "bun" => Some("bun"), _ => None,
        }
    };
    matches!((normalize(a), normalize(b)), (Some(a), Some(b)) if a == b)
}

pub(crate) fn generic_interpreter_recipe(bin: &str) -> bool {
    let (name, _) = strip_exe(base_name(bin));
    let name = name.to_ascii_lowercase();
    is_script_runtime(&name)
        || matches!(name.as_str(), "sh" | "bash" | "zsh" | "dash" | "fish" | "ksh"
            | "csh" | "tcsh" | "login" | "env" | "powershell" | "pwsh" | "cmd"
            | "conhost" | "openconsole" | "winpty" | "conpty" | "py" | "pythonw")
        || name.strip_prefix("python").is_some_and(|version| version.is_empty()
            || version.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

fn match_unconstrained_slot(recipe: &Recipe, obs: &ProcessObservation) -> Option<Slot> {
    // Native executable identity does not turn a missing invocation entry into a supported CLI.
    obs.invocation
        .argv
        .first()
        .filter(|argv0| !argv0.is_empty())?;
    let exe = lossy(&obs.identity.executable);
    let exe_base = base_name(&exe);
    if bin_eq(exe_base, &recipe.bin) {
        return Some(Slot::Native);
    }
    let argv0 = obs
        .invocation
        .argv
        .first()
        .map(|a| lossy(a))
        .unwrap_or_default();
    // Native installers that keep versioned binaries (measured: ~/.local/share/claude/versions/2.1.300
    // launched as argv0 "claude").
    let versions_dir = format!("/{}/versions/", recipe.bin);
    let versions_dir_win = format!("\\{}\\versions\\", recipe.bin);
    if bin_eq(base_name(&argv0), &recipe.bin)
        && (exe.contains(&versions_dir) || exe.contains(&versions_dir_win))
    {
        return Some(Slot::Native);
    }
    let (rt, _) = strip_exe(exe_base);
    if SCRIPT_RUNTIMES.iter().any(|r| rt.eq_ignore_ascii_case(r)) {
        // Runtime options can own operands; an operand named after an agent is not a script.
        // Unknown layouts have no supported recipe and cannot become a full-policy match.
        let idx = runtime_script_slot(&obs.invocation.argv)?;
        let script = lossy(&obs.invocation.argv[idx]);
        let sbase = base_name(&script);
        let stem = sbase
            .strip_suffix(".js")
            .or_else(|| sbase.strip_suffix(".mjs"))
            .or_else(|| sbase.strip_suffix(".cjs"))
            .unwrap_or(sbase);
        let canonical = script.replace('\\', "/");
        let pkg_hit = match recipe.bin.as_str() {
            "claude" => canonical.ends_with("/@anthropic-ai/claude-code/cli.js"),
            "codex" => canonical.ends_with("/@openai/codex/bin/codex.js"),
            "gemini" => canonical.ends_with("/@google/gemini-cli/dist/index.js"),
            _ => false,
        };
        if bin_eq(stem, &recipe.bin) || pkg_hit {
            return Some(Slot::Script(idx));
        }
    }
    None
}

/// Enumerated runtime forms used by supported native/npm launches. Values belonging to options
/// are skipped; eval/print and unrecognised runtime syntax cannot supply a script witness.
pub(crate) fn runtime_script_slot(argv: &[Vec<u8>]) -> Option<usize> {
    let mut i = 1;
    while let Some(arg) = argv.get(i) {
        let arg = std::str::from_utf8(arg).ok()?;
        if arg == "--" {
            return argv.get(i + 1).map(|_| i + 1);
        }
        if !arg.starts_with('-') {
            return Some(i);
        }
        let key = arg.split('=').next()?;
        if [
            "--require",
            "-r",
            "--import",
            "--loader",
            "--experimental-loader",
            "--conditions",
            "-C",
        ]
        .contains(&key)
        {
            if !arg.contains('=') {
                argv.get(i + 1)?;
                i += 1;
            }
        } else if [
            "--no-warnings",
            "--trace-warnings",
            "--enable-source-maps",
            "--no-deprecation",
            "--inspect",
            "--inspect-brk",
            "--experimental-strip-types",
        ]
        .contains(&key)
            || (key == "--max-old-space-size" && arg.contains('='))
        {
            // These forms either carry no operand or put it after '='.
        } else {
            return None;
        }
        i += 1;
    }
    None
}

/// The adapter arguments of an invocation (after the executable or after the script slot).
fn adapter_args(obs: &ProcessObservation, slot: Slot) -> &[Vec<u8>] {
    let argv = &obs.invocation.argv;
    match slot {
        Slot::Native => argv.get(1..).unwrap_or(&[]),
        Slot::Script(i) => argv.get(i + 1..).unwrap_or(&[]),
    }
}

/// One process of the topology snapshot taken for an attempt.
#[derive(Clone, Debug)]
pub struct Node {
    pub pid: u32,
    pub parent: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unknown {
    RootUnreadable(String),
    RootChanged,
    RootDead,
    NoCandidate,
    Ambiguous(String),
    CandidateUnreadable(String),
    CandidateVanished,
    ForegroundUnreadable,
    ContraryForeground,
    Unsupported,
    AncestryChanged,
    ValidationFailed(String),
}

impl Unknown {
    pub fn reason(&self) -> String {
        match self {
            Self::RootUnreadable(s) => format!("root_unreadable:{s}"),
            Self::RootChanged => "root_changed".into(),
            Self::RootDead => "root_dead".into(),
            Self::NoCandidate => "no_candidate".into(),
            Self::Ambiguous(s) => format!("ambiguous:{s}"),
            Self::CandidateUnreadable(s) => format!("candidate_unreadable:{s}"),
            Self::CandidateVanished => "candidate_vanished".into(),
            Self::ForegroundUnreadable => "foreground_unreadable".into(),
            Self::ContraryForeground => "contrary_foreground".into(),
            Self::Unsupported => "unsupported_platform".into(),
            Self::AncestryChanged => "ancestry_changed".into(),
            Self::ValidationFailed(s) => format!("validation:{s}"),
        }
    }
}

fn failure_reason(f: &Failure) -> String {
    match f {
        Failure::InvalidPid => "invalid_pid".into(),
        Failure::Dead => "dead".into(),
        Failure::Unknown { stage, os_error } => format!("{stage}:{os_error:?}"),
        Failure::Malformed(m) => format!("malformed:{m}"),
        Failure::Changed => "changed".into(),
        Failure::UnsupportedPlatform => "unsupported".into(),
    }
}

/// Descendants of `root` in DFS order, cycle-safe (pid reuse can create parent cycles).
pub fn descendants(nodes: &[Node], root: u32) -> Vec<(u32, u32)> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for n in nodes {
        if n.pid != n.parent {
            kids.entry(n.parent).or_default().push(n.pid);
        }
    }
    for v in kids.values_mut() {
        v.sort_unstable();
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    seen.insert(root);
    while let Some(p) = stack.pop() {
        if let Some(cs) = kids.get(&p) {
            for &c in cs.iter().rev() {
                if seen.insert(c) {
                    out.push((c, p));
                    stack.push(c);
                }
            }
        }
    }
    out
}

/// Outcome of pure selection over a snapshot: the chosen receiver pid, or Unknown.
///
/// Rules (design 4.1): candidates are strict slot matches; on Unix only those in the PTY
/// foreground group are plausible stdin receivers; matches that lie on ONE ancestor chain with
/// identical adapter arguments collapse ONLY for the supported Codex node-shim -> native recipe.
/// Native/native and script/script pairs are distinct plausible receivers even with equal argv;
/// incomparable matches, unreadable candidates and no candidate are Unknown.
pub fn select_receiver(
    recipe: &Recipe,
    tree: &[(u32, u32)],
    observed: &HashMap<u32, Result<ProcessObservation, Failure>>,
    foreground_pgid: Option<Option<u32>>,
) -> Result<u32, Unknown> {
    let parent: HashMap<u32, u32> = tree.iter().copied().collect();
    let mut matches: Vec<(u32, Slot)> = Vec::new();
    for (pid, _) in tree {
        match observed.get(pid) {
            Some(Ok(obs)) => {
                if let Some(slot) = match_slot(recipe, obs) {
                    if let Some(fg) = foreground_pgid {
                        let fg = fg.ok_or(Unknown::ForegroundUnreadable)?;
                        if obs.identity.process_group != fg {
                            continue; // not the terminal's foreground group: not a stdin receiver
                        }
                    }
                    matches.push((*pid, slot));
                }
            }
            Some(Err(Failure::Dead)) => {}
            // A process we cannot read could itself be a second receiver: refuse to guess.
            Some(Err(f)) => {
                return Err(Unknown::CandidateUnreadable(format!(
                    "{pid}:{}",
                    failure_reason(f)
                )))
            }
            None => return Err(Unknown::CandidateUnreadable(format!("{pid}:not_observed"))),
        }
    }
    if matches.is_empty() {
        return Err(Unknown::NoCandidate);
    }
    let depth = |mut p: u32| {
        let mut d = 0;
        while let Some(&q) = parent.get(&p) {
            d += 1;
            p = q;
            if d > 4096 {
                break;
            }
        }
        d
    };
    matches.sort_by_key(|(p, _)| depth(*p));
    for w in matches.windows(2) {
        let (a, sa) = w[0];
        let (b, sb) = w[1];
        let oa = observed[&a].as_ref().unwrap();
        let ob = observed[&b].as_ref().unwrap();
        let shim_exe = lossy(&oa.identity.executable);
        let (runtime, _) = strip_exe(base_name(&shim_exe));
        let supported_shim = bin_eq(&recipe.bin, "codex")
            && matches!((sa, sb), (Slot::Script(_), Slot::Native))
            && ["node", "nodejs"]
                .iter()
                .any(|r| runtime.eq_ignore_ascii_case(r))
            && parent.get(&b) == Some(&a);
        if !supported_shim || adapter_args(oa, sa) != adapter_args(ob, sb) {
            return Err(Unknown::Ambiguous(format!("{a},{b}")));
        }
    }
    Ok(matches.last().unwrap().0)
}

/// Epochs that key a binding (design 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Epochs {
    pub daemon_instance: u64,
    pub surface_instance: u64,
    pub launch: u64,
    pub conversation: u64,
    pub policy: u64,
    pub binding: u64,
}

/// Identity of a single attempted publication. A worker returns this captured ticket, never a
/// relative "current request" pointer; publication compares it with the live controller state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptTicket {
    pub epochs: Epochs,
    pub obligation_id: String,
    pub request_id: String,
    pub attempt_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationPhase {
    Pending,
    Published,
    Cancelled,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishRefusal {
    Superseded,
    NotPending,
    Cancelled,
    Expired,
    PossibleWrite,
    NativeInvalid,
    InputHeld,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeWriteCompletion {
    Exact(usize),
    Unknown,
}

/// Kept under the per-obligation controller mutex; cancel/expiry/publication mutate THIS object.
/// Explicit native-write facts are separate from Request::status (Writing precedes proof admission).
pub struct PublicationGate {
    pub ticket: AttemptTicket,
    pub budget: StartBudget,
    pub phase: PublicationPhase,
    zero_native_write: bool,
    native_write_outstanding: bool,
}

impl PublicationGate {
    pub fn new(ticket: AttemptTicket) -> Self {
        Self {
            ticket,
            budget: StartBudget::new(),
            phase: PublicationPhase::Pending,
            zero_native_write: true,
            native_write_outstanding: false,
        }
    }
    pub fn cancel(&mut self) {
        self.phase = PublicationPhase::Cancelled;
    }
    pub fn native_write_started(&mut self) {
        self.native_write_outstanding = true;
    }
    pub fn native_write_returned(&mut self, completion: NativeWriteCompletion) {
        self.native_write_outstanding = false;
        match completion {
            NativeWriteCompletion::Exact(0) => {}
            NativeWriteCompletion::Exact(_) | NativeWriteCompletion::Unknown => {
                self.zero_native_write = false
            }
        }
    }
    pub fn publish(
        &mut self,
        returned: &AttemptTicket,
        current_epochs: Epochs,
        now: Instant,
        native_valid: bool,
        input_safe: bool,
    ) -> Result<(), PublishRefusal> {
        if returned != &self.ticket || current_epochs != self.ticket.epochs {
            return Err(PublishRefusal::Superseded);
        }
        match self.phase {
            PublicationPhase::Cancelled => return Err(PublishRefusal::Cancelled),
            PublicationPhase::Expired => return Err(PublishRefusal::Expired),
            PublicationPhase::Published => return Err(PublishRefusal::NotPending),
            PublicationPhase::Pending => {}
        }
        if self.budget.expired(now) {
            self.phase = PublicationPhase::Expired;
            return Err(PublishRefusal::Expired);
        }
        if !self.zero_native_write || self.native_write_outstanding {
            return Err(PublishRefusal::PossibleWrite);
        }
        if !native_valid {
            return Err(PublishRefusal::NativeInvalid);
        }
        if !input_safe {
            return Err(PublishRefusal::InputHeld);
        }
        self.phase = PublicationPhase::Published;
        Ok(())
    }
}

/// A daemon-owned native receiver binding.
pub struct Binding {
    pub epochs: Epochs,
    pub class: ProofClass,
    pub root: Retained,
    pub agent: Retained,
    /// Root-exclusive, agent-exclusive native ancestry; every node is rechecked per call.
    pub ancestry: Vec<Retained>,
    /// Unix: the agent's process group, which must stay the PTY foreground group.
    pub agent_pgid: Option<u32>,
}

impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Binding")
            .field("epochs", &self.epochs)
            .field("class", &self.class)
            .field("root_pid", &self.root.observation().identity.pid)
            .field("agent_pid", &self.agent.observation().identity.pid)
            .field("agent_pgid", &self.agent_pgid)
            .finish()
    }
}

/// Why a per-call proof failed. Only `AgentDead` / `RootDead` are death; everything else is a
/// loss of proof that must not be read as death (S4-1: a non-death Unknown may resume).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofLoss {
    AgentDead,
    RootDead,
    Changed,
    Unknown(String),
    ContraryForeground,
    EpochMoved,
}

impl ProofLoss {
    pub fn is_death(&self) -> bool {
        matches!(self, Self::AgentDead | Self::RootDead)
    }
    /// May the SAME request resume after this loss (S4-1 PartialExact)? Only a transient loss of
    /// proof of an unchanged binding: death, replacement (`Changed`) and an epoch move are not.
    pub fn resumable(&self) -> bool {
        matches!(self, Self::Unknown(_) | Self::ContraryForeground)
    }
    pub fn reason(&self) -> String {
        match self {
            Self::AgentDead => "agent_dead".into(),
            Self::RootDead => "root_dead".into(),
            Self::Changed => "identity_changed".into(),
            Self::Unknown(s) => format!("unknown:{s}"),
            Self::ContraryForeground => "contrary_foreground".into(),
            Self::EpochMoved => "epoch_moved".into(),
        }
    }
}

impl Binding {
    /// The per-OS-call proof (design 4.1 "Each per-write proof must recheck the expected executable
    /// identity", 4.4 step 3). `foreground` reads the PTY foreground group NOW (Unix only).
    pub fn check(&self, foreground: &dyn Fn() -> Option<u32>) -> Result<(), ProofLoss> {
        match self.root.recheck() {
            Ok(()) => {}
            Err(Failure::Dead) => return Err(ProofLoss::RootDead),
            Err(Failure::Changed) => return Err(ProofLoss::Changed),
            Err(f) => return Err(ProofLoss::Unknown(format!("root:{}", failure_reason(&f)))),
        }
        for ancestor in &self.ancestry {
            match ancestor.recheck() {
                Ok(()) => {}
                Err(Failure::Changed) => return Err(ProofLoss::Changed),
                Err(f) => {
                    return Err(ProofLoss::Unknown(format!(
                        "ancestor:{}",
                        failure_reason(&f)
                    )))
                }
            }
        }
        match self.agent.recheck() {
            Ok(()) => {}
            Err(Failure::Dead) => return Err(ProofLoss::AgentDead),
            Err(Failure::Changed) => return Err(ProofLoss::Changed),
            Err(f) => return Err(ProofLoss::Unknown(format!("agent:{}", failure_reason(&f)))),
        }
        if let Some(pg) = self.agent_pgid {
            match foreground() {
                Some(fg) if fg == pg => {}
                Some(_) => return Err(ProofLoss::ContraryForeground),
                None => return Err(ProofLoss::Unknown("foreground_unreadable".into())),
            }
        }
        Ok(())
    }

    /// (pid, birth) of the bound agent — used to seal a dead identity so it is never revived.
    pub fn agent_identity(&self) -> (u32, u64, u64) {
        let i = &self.agent.observation().identity;
        (i.pid, i.birth_seconds, i.birth_microseconds)
    }
}

/// Ordered root-exclusive ancestry of the selected receiver. Native edge validation is mandatory
/// before retaining/publishing a chain. Unrelated dead descendants are not ancestors.
fn selected_chain(
    root: u32,
    selected: u32,
    tree: &[(u32, u32)],
    observed: &HashMap<u32, Result<ProcessObservation, Failure>>,
) -> Result<Vec<u32>, Unknown> {
    let parents: HashMap<u32, u32> = tree.iter().copied().collect();
    let root_obs = observed
        .get(&root)
        .and_then(|r| r.as_ref().ok())
        .ok_or(Unknown::AncestryChanged)?;
    let birth = |o: &ProcessObservation| (o.identity.birth_seconds, o.identity.birth_microseconds);
    if root_obs.identity.pid != root || birth(root_obs) == (0, 0) {
        return Err(Unknown::AncestryChanged);
    }
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut pid = selected;
    while pid != root {
        if !seen.insert(pid) {
            return Err(Unknown::CandidateVanished);
        }
        let parent = *parents.get(&pid).ok_or(Unknown::AncestryChanged)?;
        let native = observed
            .get(&pid)
            .and_then(|r| r.as_ref().ok())
            .ok_or(Unknown::AncestryChanged)?;
        let parent_obs = observed
            .get(&parent)
            .and_then(|r| r.as_ref().ok())
            .ok_or(Unknown::AncestryChanged)?;
        if native.identity.pid != pid
            || native.identity.parent != parent
            || parent_obs.identity.pid != parent
            || birth(native) == (0, 0)
            || birth(parent_obs) == (0, 0)
            || birth(parent_obs) > birth(native)
        {
            return Err(Unknown::AncestryChanged);
        }
        chain.push(pid);
        pid = parent;
    }
    chain.reverse();
    Ok(chain)
}

/// Inputs to one discovery attempt, all owned so it can run on an executor thread.
pub struct DiscoveryInput {
    pub root_pid: u32,
    pub recipe: Recipe,
    pub class: ProofClass,
    pub epochs: Epochs,
    /// Sealed (proven dead) agent identities of this surface: never rebind to one of them.
    pub sealed: Vec<(u32, u64, u64)>,
    pub snapshot: Box<dyn Fn() -> Vec<Node> + Send>,
    pub foreground: Box<dyn Fn() -> Option<u32> + Send>,
    pub observe: Box<dyn Fn(u32) -> Result<ProcessObservation, Failure> + Send>,
    pub retain: Box<dyn Fn(u32) -> Result<Retained, Failure> + Send>,
}

/// One attempt's collection. Classification reads this same material before fixing a policy;
/// binding then consumes it without a second topology refresh or initial observation pass.
pub struct NativeCollection {
    pub root: Retained,
    pub tree: Vec<(u32, u32)>,
    pub observed: HashMap<u32, Result<ProcessObservation, Failure>>,
    pub foreground: Option<Option<u32>>,
}

pub fn collect(input: &DiscoveryInput) -> Result<NativeCollection, Unknown> {
    if !native_backend_available() {
        return Err(Unknown::Unsupported);
    }
    let root = (input.retain)(input.root_pid).map_err(|f| match f {
        Failure::Dead => Unknown::RootDead,
        f => Unknown::RootUnreadable(failure_reason(&f)),
    })?;
    let nodes = (input.snapshot)();
    let mut tree = vec![(input.root_pid, root.observation().identity.parent)];
    tree.extend(descendants(&nodes, input.root_pid));
    let mut observed = HashMap::new();
    observed.insert(input.root_pid, Ok(root.observation().clone()));
    for &(pid, _) in &tree[1..] {
        observed.insert(pid, (input.observe)(pid));
    }
    let foreground = if cfg!(unix) {
        Some((input.foreground)())
    } else {
        None
    };
    Ok(NativeCollection {
        root,
        tree,
        observed,
        foreground,
    })
}

/// Identify one declaration from a catalog frozen before this native attempt. A selected
/// declaration retains its precomputed class even when later binding fails.
pub fn classify_collected(
    catalog: &[ReceiverPolicy],
    collected: &NativeCollection,
) -> Result<ReceiverPolicy, Unknown> {
    let mut selected: Option<&ReceiverPolicy> = None;
    for declaration in catalog {
        match select_receiver(
            &declaration.recipe,
            &collected.tree,
            &collected.observed,
            collected.foreground,
        ) {
            Ok(pid) => {
                // Classification fixes a declaration, so its candidate must already belong to
                // this root by coherent native ancestry. This is not success-dependent downgrading.
                selected_chain(
                    collected.root.observation().identity.pid,
                    pid,
                    &collected.tree,
                    &collected.observed,
                )?;
                if selected.is_some_and(|old| old != declaration) {
                    return Err(Unknown::Ambiguous("multiple_declarations".into()));
                }
                selected = Some(declaration);
            }
            Err(Unknown::NoCandidate) => {}
            Err(unknown) => return Err(unknown),
        }
    }
    selected.cloned().ok_or(Unknown::NoCandidate)
}

/// Select and retain only after the declaration is fixed. Recheck the complete native chain and
/// current foreground after all potentially slow queries, still inside the same executor attempt.
pub fn bind_collected(
    input: &DiscoveryInput,
    collected: NativeCollection,
) -> Result<Binding, Unknown> {
    let NativeCollection {
        root,
        tree,
        observed,
        foreground,
    } = collected;
    let pid = select_receiver(&input.recipe, &tree, &observed, foreground)?;
    let chain = selected_chain(input.root_pid, pid, &tree, &observed)?;
    let mut ancestry = Vec::new();
    for &ancestor_pid in chain.iter().filter(|&&p| p != pid) {
        let retained = (input.retain)(ancestor_pid).map_err(|_| Unknown::AncestryChanged)?;
        if Some(retained.observation()) != observed.get(&ancestor_pid).and_then(|r| r.as_ref().ok())
        {
            return Err(Unknown::AncestryChanged);
        }
        ancestry.push(retained);
    }
    let agent = (input.retain)(pid).map_err(|f| match f {
        Failure::Dead => Unknown::CandidateVanished,
        f => Unknown::CandidateUnreadable(format!("{pid}:{}", failure_reason(&f))),
    })?;
    if Some(agent.observation()) != observed.get(&pid).and_then(|r| r.as_ref().ok()) {
        return Err(Unknown::CandidateVanished);
    }
    let id = agent.observation().identity.clone();
    if input
        .sealed
        .contains(&(id.pid, id.birth_seconds, id.birth_microseconds))
    {
        return Err(Unknown::Ambiguous("sealed_identity".into()));
    }
    let bound = Binding {
        epochs: input.epochs,
        class: input.class,
        root,
        agent,
        ancestry,
        agent_pgid: cfg!(unix).then_some(id.process_group),
    };
    bound.check(&*input.foreground).map_err(|loss| match loss {
        ProofLoss::RootDead => Unknown::RootDead,
        ProofLoss::AgentDead => Unknown::CandidateVanished,
        other => Unknown::ValidationFailed(other.reason()),
    })?;
    Ok(bound)
}

pub fn discover(input: &DiscoveryInput) -> Result<Binding, Unknown> {
    bind_collected(input, collect(input)?)
}

// ---------------------------------------------------------------------------------------------
// Bounded discovery executor (design 4.1 "Bounded observation contract").

/// Why an admission did not start a native query. Neither consumes a start (S3-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartRefusal {
    SlotBusy,
    Saturated,
    BudgetExhausted,
    TooSoon,
    WindowExpired,
    AdmissionBusy,
    Closed,
}

#[derive(Default, Debug)]
pub struct Counters {
    pub admissions: AtomicU64,
    pub admitted: AtomicU64,
    pub admission_busy: AtomicU64,
    pub pre_start_refused: AtomicU64,
    pub started: AtomicU64,
    pub slot_busy: AtomicU64,
    pub saturated: AtomicU64,
    pub caller_deadlines: AtomicU64,
    pub late_completions: AtomicU64,
    pub late_after_window: AtomicU64,
}

pub type Job = Box<dyn FnOnce() + Send>;
pub type Preparation = Box<dyn FnOnce() -> Option<Job> + Send>;

/// Fixed pool of `DISCOVERY_WORKERS` threads, one in-flight query per surface slot, fair FIFO.
pub struct Executor {
    inner: Arc<ExecInner>,
}

struct ExecInner {
    state: Mutex<ExecState>,
    cv: Condvar,
    pub counters: Counters,
    capacity: usize,
}

struct Scheduled {
    prepare: Preparation,
    finished: Option<Job>,
}

struct ExecState {
    queue: VecDeque<(u64, Scheduled)>,
    busy: HashSet<u64>,
    running: usize,
    closed: bool,
}

/// Only construction owns these handles. Before publication, no job can enter this pool.
struct SpawnCleanup {
    inner: Arc<ExecInner>,
    workers: Vec<std::thread::JoinHandle<()>>,
    armed: bool,
}

impl Drop for SpawnCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return; // Successful construction detaches the handles when the fields are dropped.
        }
        {
            let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            st.closed = true;
        }
        self.inner.cv.notify_all();
        // Construction has not published an Executor: these workers can only be idle/startup.
        // Do not hold state while joining, and do not reuse this join path from Executor::drop.
        for worker in self.workers.drain(..) {
            // Preserve the constructor's original error/unwind even if a worker also panicked.
            let _ = worker.join();
        }
    }
}

// Same worker body as the observed Executor::new closure. No admission/lifecycle change here.
fn run_discovery_worker(inner: Arc<ExecInner>) {
    loop {
        let (slot, job) = {
            let mut st = inner.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if st.closed {
                    return;
                }
                if let Some(j) = st.queue.pop_front() {
                    st.running += 1;
                    break j;
                }
                st = inner.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        let Scheduled { prepare, finished } = job;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(prepare)) {
            Ok(Some(native)) => {
                inner.counters.started.fetch_add(1, Ordering::Relaxed);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(native));
            }
            _ => {
                inner
                    .counters
                    .pre_start_refused
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        {
            let mut st = inner.state.lock().unwrap_or_else(|e| e.into_inner());
            st.running -= 1;
            st.busy.remove(&slot);
        }
        if let Some(finished) = finished {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(finished));
        }
    }
}

impl Executor {
    /// Compatibility constructor. A spawn error still panics here, after partial-pool cleanup.
    /// New runtime/controller wiring must use try_new and handle its error explicitly.
    pub fn new(workers: usize) -> Executor {
        Self::try_new(workers).expect("spawn discovery worker")
    }

    /// Returns a usable full pool or an error after all successfully spawned idle workers exit.
    /// Zero workers preserves the former empty/saturated-pool behavior; no new policy is added.
    pub fn try_new(workers: usize) -> std::io::Result<Executor> {
        Self::try_new_with_spawn(workers, |i, task| {
            std::thread::Builder::new()
                .name(format!("cysd-discovery-{i}"))
                .spawn(task)
        })
    }

    /// Private trusted test seam, not a caller-supplied runtime extension.
    /// Factory contract: Err/panic means it retained/launched no supplied task; Ok returns the
    /// sole handle for a worker that invokes the supplied task exactly once. The real factory
    /// above delegates directly to Builder::spawn. Tests may wrap the task to observe exit.
    fn try_new_with_spawn(
        workers: usize,
        mut spawn: impl FnMut(usize, Job) -> std::io::Result<std::thread::JoinHandle<()>>,
    ) -> std::io::Result<Executor> {
        let inner = Arc::new(ExecInner {
            state: Mutex::new(ExecState {
                queue: VecDeque::new(),
                busy: HashSet::new(),
                running: 0,
                closed: false,
            }),
            cv: Condvar::new(),
            counters: Counters::default(),
            capacity: workers,
        });
        let mut cleanup = SpawnCleanup {
            inner: inner.clone(),
            // Reserve all handle storage before the first thread exists.
            workers: Vec::with_capacity(workers),
            armed: true,
        };
        for i in 0..workers {
            let worker_inner = inner.clone();
            let handle = spawn(i, Box::new(move || run_discovery_worker(worker_inner)))?;
            cleanup.workers.push(handle);
        }
        // No fallible operation between disarming and publication. Handle Drop detaches only.
        cleanup.armed = false;
        Ok(Executor { inner })
    }

    pub fn counters(&self) -> &Counters {
        &self.inner.counters
    }

    /// Admit a native query for `slot` (a surface instance). The slot stays occupied until the
    /// job RETURNS — a caller deadline never frees it and never starts a second query (S3-2).
    pub fn try_start(&self, slot: u64, job: Job) -> Result<(), StartRefusal> {
        self.try_start_guarded(slot, Box::new(move || Some(job)), None)
    }

    /// `prepare` runs on the selected worker and returns native work only after its locked
    /// obligation gate permits an actual start. A refusal/queued expiry consumes no native start.
    /// `finished` runs after slot release, even when preparation or native observation panics.
    pub fn try_start_guarded(
        &self,
        slot: u64,
        prepare: Preparation,
        finished: Option<Job>,
    ) -> Result<(), StartRefusal> {
        self.inner
            .counters
            .admissions
            .fetch_add(1, Ordering::Relaxed);
        let mut st = match self.inner.state.try_lock() {
            Ok(st) => st,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.inner
                    .counters
                    .admission_busy
                    .fetch_add(1, Ordering::Relaxed);
                return Err(StartRefusal::AdmissionBusy);
            }
        };
        if st.closed {
            return Err(StartRefusal::Closed);
        }
        if st.busy.contains(&slot) {
            self.inner
                .counters
                .slot_busy
                .fetch_add(1, Ordering::Relaxed);
            return Err(StartRefusal::SlotBusy);
        }
        if st.running + st.queue.len() >= self.inner.capacity {
            self.inner
                .counters
                .saturated
                .fetch_add(1, Ordering::Relaxed);
            return Err(StartRefusal::Saturated);
        }
        st.busy.insert(slot);
        st.queue.push_back((slot, Scheduled { prepare, finished }));
        self.inner.counters.admitted.fetch_add(1, Ordering::Relaxed);
        drop(st);
        self.inner.cv.notify_one();
        Ok(())
    }

    pub fn slot_busy(&self, slot: u64) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .busy
            .contains(&slot)
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        let queued = {
            let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            st.closed = true;
            let queued = std::mem::take(&mut st.queue);
            for (slot, _) in &queued {
                st.busy.remove(slot);
            }
            queued
        };
        self.inner.cv.notify_all();
        // No mutex over captured-object destruction, and no join over possibly blocked native I/O.
        // Running jobs keep their own slot/inner alive until return; idle workers exit immediately.
        drop(queued);
    }
}

/// Start budget of ONE proof obligation (design 4.1/4.6): at most four starts, >=250 ms apart, at
/// most one before `eligible_at` (settle/modal prefetch) so three remain for the proof window.
#[derive(Debug, Clone)]
pub struct StartBudget {
    pub starts: u32,
    pub last_start: Option<Instant>,
    pub eligible_at: Option<Instant>,
    pub deadline: Option<Instant>,
}

impl StartBudget {
    pub fn new() -> Self {
        StartBudget {
            starts: 0,
            last_start: None,
            eligible_at: None,
            deadline: None,
        }
    }

    /// Fix the absolute proof window once; later calls never move it (no reset by retries,
    /// slot-busy refusals or late completions).
    pub fn open_window(&mut self, eligible_at: Instant, window: Duration) {
        if self.eligible_at.is_none() {
            self.eligible_at = Some(eligible_at);
            self.deadline = Some(eligible_at + window);
        }
    }

    pub fn may_start(&self, now: Instant) -> Result<(), StartRefusal> {
        if self.expired(now) {
            return Err(StartRefusal::WindowExpired);
        }
        if self.starts >= MAX_STARTS_PER_OBLIGATION {
            return Err(StartRefusal::BudgetExhausted);
        }
        if let Some(t) = self.last_start {
            if now.saturating_duration_since(t) < Duration::from_millis(MIN_START_SPACING_MS) {
                return Err(StartRefusal::TooSoon);
            }
        }
        let before_eligible = self.eligible_at.map_or(true, |e| now < e);
        if before_eligible && self.starts >= 1 {
            return Err(StartRefusal::BudgetExhausted); // keep three starts for eligibility
        }
        Ok(())
    }

    /// Record an ACTUAL start on the worker immediately before native observation, never at enqueue.
    pub fn begin(&mut self, now: Instant) -> Result<(), StartRefusal> {
        self.may_start(now)?;
        self.note_started(now);
        Ok(())
    }

    fn note_started(&mut self, now: Instant) {
        self.starts += 1;
        self.last_start = Some(now);
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|d| now > d)
    }
}

impl Default for StartBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cys::process_observation::{Identity, Invocation};

    fn obs(pid: u32, pgid: u32, exe: &str, argv: &[&str]) -> ProcessObservation {
        ProcessObservation {
            identity: Identity {
                pid,
                parent: 0,
                process_group: pgid,
                birth_seconds: 1_000 + pid as u64,
                birth_microseconds: 7,
                pointer_width: 8,
                executable: exe.as_bytes().to_vec(),
            },
            invocation: Invocation {
                launch_path: exe.as_bytes().to_vec(),
                argv: argv.iter().map(|a| a.as_bytes().to_vec()).collect(),
            },
        }
    }

    fn recipe(bin: &str) -> Recipe {
        Recipe { bin: bin.into(), declared_invocation: None }
    }

    fn prepare_when_uncontended(
        ex: &Executor,
        slot: u64,
        mut make: impl FnMut() -> (Preparation, Option<Job>),
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (prepare, finished) = make();
            match ex.try_start_guarded(slot, prepare, finished) {
                Ok(()) => return,
                Err(StartRefusal::AdmissionBusy) if Instant::now() < deadline => {
                    std::thread::yield_now()
                }
                other => panic!("fixture guarded admission: {other:?}"),
            }
        }
    }

    #[test]
    fn c8e_native_match_requires_nonempty_argv_zero() {
        for args in [vec![], vec![""]] {
            assert_eq!(
                match_slot(&recipe("claude"), &obs(10, 10, "/u/claude", &args)),
                None,
                "an executable name cannot stand in for a missing invocation entry"
            );
        }
        assert_eq!(
            match_slot(&recipe("claude"), &obs(10, 10, "/u/claude", &["claude"])),
            Some(Slot::Native)
        );
    }

    #[test]
    fn c8e_codex_shim_collapse_rejects_unknown_intermediaries() {
        let (tree, map) = table(vec![
            (10, 1, obs(10, 10, "/u/node", &["node", "/u/codex", "--x"])),
            (
                11,
                10,
                obs(11, 10, "/u/unknown-wrapper", &["unknown-wrapper"]),
            ),
            (12, 11, obs(12, 10, "/u/codex", &["codex", "--x"])),
        ]);
        assert!(
            matches!(
                select_receiver(&recipe("codex"), &tree, &map, Some(Some(10))),
                Err(Unknown::Ambiguous(_))
            ),
            "a supported shim cannot confer support on an unrecognized intermediary"
        );
    }

    #[test]
    fn c8e_codex_shim_collapse_requires_its_supported_runtime() {
        let (tree, map) = table(vec![
            (10, 1, obs(10, 10, "/u/bun", &["bun", "/u/codex", "--x"])),
            (11, 10, obs(11, 10, "/u/codex", &["codex", "--x"])),
        ]);
        assert!(
            matches!(
                select_receiver(&recipe("codex"), &tree, &map, Some(Some(10))),
                Err(Unknown::Ambiguous(_))
            ),
            "the measured node shim rule cannot silently authorize a different runtime"
        );
    }

    #[test]
    fn c8e_selected_chain_rejects_a_parent_born_after_its_child() {
        for changed_parent in [1, 10] {
            let (tree, mut map) = table(vec![
                (1, 0, obs(1, 1, "/bin/sh", &["sh"])),
                (10, 1, obs(10, 10, "/u/node", &["node", "/u/codex"])),
                (11, 10, obs(11, 10, "/u/codex", &["codex"])),
            ]);
            assert_eq!(selected_chain(1, 11, &tree, &map), Ok(vec![10, 11]));
            map.get_mut(&changed_parent)
                .unwrap()
                .as_mut()
                .unwrap()
                .identity
                .birth_seconds = 9_999;
            assert!(selected_chain(1, 11, &tree, &map).is_err(),
                "matching parent PID is insufficient when its creation identity contradicts ancestry");
        }
    }

    #[test]
    fn c8e_missing_native_birth_is_not_full_identity() {
        let (tree, mut map) = table(vec![(1, 0, obs(1, 1, "/u/claude", &["claude"]))]);
        let id = &mut map.get_mut(&1).unwrap().as_mut().unwrap().identity;
        id.birth_seconds = 0;
        id.birth_microseconds = 0;
        assert!(
            selected_chain(1, 1, &tree, &map).is_err(),
            "missing root creation identity cannot bind a full receiver"
        );
    }

    #[test]
    fn c8e_unknown_native_completion_permanently_taints_zero_write_proof() {
        let at = Instant::now();
        let mut gate = publication_fixture(at);
        let key = gate.ticket.clone();
        gate.native_write_started();
        gate.native_write_returned(NativeWriteCompletion::Unknown);
        gate.native_write_started();
        gate.native_write_returned(NativeWriteCompletion::Exact(0));
        assert!(
            gate.publish(&key, key.epochs, at, true, true).is_err(),
            "a later exact zero cannot erase an earlier unknown accepted-byte result"
        );
    }

    #[test]
    fn c8e_every_returned_epoch_must_match_while_live_epochs_stay_fixed() {
        let at = Instant::now();
        for axis in 0..6 {
            let mut gate = publication_fixture(at);
            let live = gate.ticket.epochs;
            let mut key = gate.ticket.clone();
            match axis {
                0 => key.epochs.daemon_instance += 1,
                1 => key.epochs.surface_instance += 1,
                2 => key.epochs.launch += 1,
                3 => key.epochs.conversation += 1,
                4 => key.epochs.policy += 1,
                5 => key.epochs.binding += 1,
                _ => unreachable!(),
            }
            assert!(
                gate.publish(&key, live, at, true, true).is_err(),
                "returned ticket epoch axis {axis} must be checked independently"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn c8e_real_same_pid_exec_invalidates_the_retained_invocation() {
        use crate::state::HideConsole;
        use std::io::{BufRead, Write};
        use std::process::{Command, Stdio};
        struct Owned {
            child: std::process::Child,
            reaped: bool,
        }
        impl Drop for Owned {
            fn drop(&mut self) {
                if !self.reaped {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                }
            }
        }
        let mut child = Owned {
            child: Command::new("/bin/sh")
                .hide_console()
                .args([
                    "-c",
                    "printf 'READY\n'; IFS= read -r trigger; exec /bin/cat",
                ])
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
            reaped: false,
        };
        let mut out = std::io::BufReader::new(child.child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        assert_eq!(line, "READY\n");
        let original = Retained::open(child.child.id()).unwrap();
        let birth = (
            original.observation().identity.birth_seconds,
            original.observation().identity.birth_microseconds,
        );
        assert_eq!(original.recheck(), Ok(()));
        child
            .child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"exec\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let changed = loop {
            let observed = cys::process_observation::observe(child.child.id()).unwrap();
            if observed.identity.executable.ends_with(b"/cat") {
                break observed;
            }
            assert!(
                Instant::now() < deadline,
                "owned shell must reach its actual exec"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(
            (
                changed.identity.birth_seconds,
                changed.identity.birth_microseconds
            ),
            birth
        );
        assert_eq!(
            original.recheck(),
            Err(Failure::Changed),
            "same PID and birth do not preserve executable/invocation identity"
        );
        drop(child.child.stdin.take());
        let status = child.child.wait().unwrap();
        child.reaped = true;
        assert!(status.success());
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn collection_fixture() -> (DiscoveryInput, ReceiverPolicy) {
        let pid = std::process::id();
        let observed = cys::process_observation::observe(pid).unwrap();
        let executable = lossy(&observed.identity.executable);
        let pgid = observed.identity.process_group;
        // This is a mechanism fixture with a declared native recipe, not a shipped CLI compatibility claim.
        let policy = ReceiverPolicy {
            policy_epoch: 1,
            declaration: "frozen test declaration".into(),
            config_digest: "frozen-before-collection".into(),
            adapter: "test-native".into(),
            recipe: recipe(base_name(&executable)),
            origin: RegistrationOrigin::HandObserved,
            class: ProofClass::FullRequired,
            weak_risk: None,
        };
        (
            DiscoveryInput {
                root_pid: pid,
                recipe: policy.recipe.clone(),
                class: policy.class,
                epochs: Epochs::default(),
                sealed: vec![],
                snapshot: Box::new(Vec::new),
                foreground: Box::new(move || Some(pgid)),
                observe: Box::new(cys::process_observation::observe),
                retain: Box::new(Retained::open),
            },
            policy,
        )
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn c8f_classification_and_binding_share_one_collection() {
        let (mut input, policy) = collection_fixture();
        let snapshots = Arc::new(AtomicU64::new(0));
        let observes = Arc::new(AtomicU64::new(0));
        let (sp, ob, root) = (snapshots.clone(), observes.clone(), input.root_pid);
        input.snapshot = Box::new(move || {
            assert_eq!(
                sp.fetch_add(1, Ordering::Relaxed),
                0,
                "classification and binding must not refresh topology twice"
            );
            vec![Node {
                pid: u32::MAX,
                parent: root,
            }]
        });
        input.observe = Box::new(move |pid| {
            assert_eq!(pid, u32::MAX);
            ob.fetch_add(1, Ordering::Relaxed);
            Err(Failure::Dead)
        });
        let collected = collect(&input).unwrap();
        let chosen = classify_collected(&[policy.clone()], &collected)
            .expect("a unique frozen declaration must classify");
        assert_eq!(chosen, policy);
        input.recipe = chosen.recipe.clone();
        input.class = chosen.class;
        let binding = bind_collected(&input, collected).unwrap();
        assert_eq!(binding.agent_identity().0, root);
        assert_eq!(snapshots.load(Ordering::Relaxed), 1);
        assert_eq!(
            observes.load(Ordering::Relaxed),
            1,
            "one initial read per descendant in this attempt"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn c8f_ambiguous_catalog_never_picks_the_first_declaration() {
        let (input, first) = collection_fixture();
        let mut second = first.clone();
        second.adapter = "other-declaration".into();
        let c = collect(&input).unwrap();
        assert!(
            matches!(
                classify_collected(&[first, second], &c),
                Err(Unknown::Ambiguous(_))
            ),
            "multiple matching declarations are ambiguous, not a first-match policy"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn c8f_fixed_class_survives_a_later_binding_failure() {
        let (mut input, policy) = collection_fixture();
        let c = collect(&input).unwrap();
        let chosen = classify_collected(&[policy], &c).unwrap();
        input.retain = Box::new(|_| {
            Err(Failure::Unknown {
                stage: "injected-denial",
                os_error: Some(13),
            })
        });
        assert!(bind_collected(&input, c).is_err());
        assert_eq!(
            chosen.class,
            ProofClass::FullRequired,
            "denied binding does not change the already fixed class"
        );
        assert_eq!(chosen.weak_risk, None);
    }

    #[cfg(unix)]
    #[test]
    fn c8f_classification_uses_the_collected_foreground() {
        let (mut input, policy) = collection_fixture();
        let pg = cys::process_observation::observe(input.root_pid)
            .unwrap()
            .identity
            .process_group;
        input.foreground = Box::new(move || Some(pg.wrapping_add(1)));
        let c = collect(&input).unwrap();
        assert!(
            classify_collected(&[policy], &c).is_err(),
            "classification cannot ignore contrary collected foreground ownership"
        );
    }

    #[cfg(unix)]
    #[test]
    fn c8f_foreground_is_revalidated_after_native_collection() {
        let (mut input, _) = collection_fixture();
        let pg = cys::process_observation::observe(input.root_pid)
            .unwrap()
            .identity
            .process_group;
        let reads = Arc::new(AtomicU64::new(0));
        let r = reads.clone();
        input.foreground = Box::new(move || {
            Some(if r.fetch_add(1, Ordering::Relaxed) == 0 {
                pg
            } else {
                pg.wrapping_add(1)
            })
        });
        assert!(
            discover(&input).is_err(),
            "a contrary foreground after collection must stop publication"
        );
        assert_eq!(
            reads.load(Ordering::Relaxed),
            2,
            "one collected observation and one final targeted validation"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn c8g_classification_requires_native_ancestry_before_fixing_policy() {
        // A controlled native-observation record: classification must validate its provenance
        // even though later Retained::open would also refuse a non-existent/moved candidate.
        for contradiction in ["parent", "birth"] {
            let (input, mut policy) = collection_fixture();
            let mut c = collect(&input).unwrap();
            let root = c.root.observation();
            let child_pid = i32::MAX as u32;
            let mut child = obs(
                child_pid,
                root.identity.process_group,
                "/fixture/child-cli",
                &["child-cli"],
            );
            child.identity.parent = input.root_pid;
            child.identity.birth_seconds = root.identity.birth_seconds + 1;
            child.identity.birth_microseconds = root.identity.birth_microseconds;
            c.tree.push((child_pid, input.root_pid));
            c.observed.insert(child_pid, Ok(child));
            policy.recipe = recipe("child-cli");
            policy.adapter = "classified-child".into();
            assert_eq!(
                classify_collected(&[policy.clone()], &c).unwrap(),
                policy,
                "valid child provenance classifies its frozen declaration"
            );
            let child = c.observed.get_mut(&child_pid).unwrap().as_mut().unwrap();
            if contradiction == "parent" {
                child.identity.parent = input.root_pid.wrapping_add(1);
            } else {
                child.identity.birth_seconds = root.identity.birth_seconds.saturating_sub(1);
            }
            assert!(
                classify_collected(&[policy], &c).is_err(),
                "contradictory native {contradiction} must not become a fixed adapter policy"
            );
        }
    }

    fn publication_fixture(at: Instant) -> PublicationGate {
        let mut gate = PublicationGate::new(AttemptTicket {
            epochs: Epochs {
                daemon_instance: 1,
                surface_instance: 2,
                launch: 3,
                conversation: 4,
                policy: 5,
                binding: 6,
            },
            obligation_id: "obligation".into(),
            request_id: "request".into(),
            attempt_id: 7,
        });
        gate.budget.open_window(at, Duration::from_millis(2_000));
        gate
    }

    #[test]
    fn c8h_start_budget_rejects_fifth_without_mutation() {
        let at = Instant::now();
        let mut budget = StartBudget::new();
        budget.open_window(at, Duration::from_millis(2_000));
        for n in 0..4 {
            budget.begin(at + Duration::from_millis(n * 250)).unwrap();
        }
        assert_eq!(budget.starts, 4);
        assert_eq!(
            budget.begin(at + Duration::from_millis(1_000)),
            Err(StartRefusal::BudgetExhausted),
            "fifth start must be refused after four properly spaced starts"
        );
        assert_eq!(budget.starts, 4, "refusal cannot consume another start");
    }

    #[test]
    fn c8d_late_in_window_completion_publishes_only_once() {
        let at = Instant::now();
        for ms in [300, 1_500, 2_000] {
            let mut gate = publication_fixture(at);
            let key = gate.ticket.clone();
            assert_eq!(
                gate.publish(&key, key.epochs, at + Duration::from_millis(ms), true, true),
                Ok(())
            );
            assert!(
                gate.publish(&key, key.epochs, at + Duration::from_millis(ms), true, true)
                    .is_err(),
                "one native completion can publish only once"
            );
            assert_eq!(gate.phase, PublicationPhase::Published);
        }
    }

    #[test]
    fn c8d_publication_uses_time_after_native_validation() {
        let at = Instant::now();
        let mut gate = publication_fixture(at);
        let key = gate.ticket.clone();
        let validation_started = at + Duration::from_millis(1_999);
        assert!(!gate.budget.expired(validation_started));
        assert!(
            gate.publish(
                &key,
                key.epochs,
                at + Duration::from_millis(2_001),
                true,
                true
            )
            .is_err(),
            "validation begun in-window cannot publish after the absolute deadline"
        );
        assert_eq!(
            gate.budget.deadline,
            Some(at + Duration::from_millis(2_000))
        );
    }

    #[test]
    fn c8d_every_instance_and_epoch_is_part_of_publication_identity() {
        let at = Instant::now();
        for axis in 0..6 {
            let mut gate = publication_fixture(at);
            let key = gate.ticket.clone();
            let mut live = key.epochs;
            match axis {
                0 => live.daemon_instance += 1,
                1 => live.surface_instance += 1,
                2 => live.launch += 1,
                3 => live.conversation += 1,
                4 => live.policy += 1,
                5 => live.binding += 1,
                _ => unreachable!(),
            }
            assert!(
                gate.publish(&key, live, at, true, true).is_err(),
                "changed live identity axis {axis} must reject stale completion"
            );
        }
        for axis in 0..3 {
            let mut gate = publication_fixture(at);
            let mut key = gate.ticket.clone();
            let epochs = key.epochs;
            match axis {
                0 => key.obligation_id.push('x'),
                1 => key.request_id.push('x'),
                2 => key.attempt_id += 1,
                _ => unreachable!(),
            }
            assert!(
                gate.publish(&key, epochs, at, true, true).is_err(),
                "changed ticket identity axis {axis} must reject stale completion"
            );
        }
    }

    #[test]
    fn c8d_cancellation_and_native_or_input_failure_prevent_publication() {
        let at = Instant::now();
        for mode in 0..3 {
            let mut gate = publication_fixture(at);
            let key = gate.ticket.clone();
            if mode == 0 {
                gate.cancel();
            }
            assert!(gate.publish(&key, key.epochs, at, mode != 1, mode != 2).is_err(),
                "cancelled, native-invalid or input-held completion cannot authorize a write (mode {mode})");
        }
    }

    #[test]
    fn c8d_zero_body_offset_does_not_prove_zero_native_writes() {
        let at = Instant::now();
        for accepted in [0, 1, 6] {
            let mut gate = publication_fixture(at);
            let key = gate.ticket.clone();
            gate.native_write_started();
            if accepted != 0 {
                gate.native_write_returned(NativeWriteCompletion::Exact(accepted));
            }
            assert!(gate.publish(&key, key.epochs, at, true, true).is_err(),
                "outstanding native call or accepted control/fence bytes forbids zero-write publication");
        }
    }

    #[test]
    fn c8c_prestart_refusal_consumes_no_native_query() {
        let ex = Executor::new(1);
        let (entered, reached) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let wait = Arc::new(Mutex::new(Some(wait)));
        let (finished, done) = std::sync::mpsc::channel();
        prepare_when_uncontended(&ex, 9, || {
            let entered = entered.clone();
            let wait = wait.clone();
            let finished = finished.clone();
            (
                Box::new(move || {
                    entered.send(()).unwrap();
                    wait.lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                    None // cancellation/expiry/generation check refused before the first native query
                }),
                Some(Box::new(move || finished.send(()).unwrap())),
            )
        });
        reached.recv_timeout(Duration::from_secs(10)).unwrap();
        let before = ex.counters().started.load(Ordering::Relaxed);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(before, 0, "preparation is not an actual native query");
        assert_eq!(
            ex.counters().started.load(Ordering::Relaxed),
            0,
            "pre-start refusal consumes no native start"
        );
    }

    #[test]
    fn c8c_completion_callback_runs_after_surface_slot_release() {
        let ex = Arc::new(Executor::new(1));
        let weak = Arc::downgrade(&ex);
        let (tx, rx) = std::sync::mpsc::channel();
        prepare_when_uncontended(&ex, 3, || {
            let weak = weak.clone();
            let tx = tx.clone();
            (
                Box::new(|| Some(Box::new(|| {}))),
                Some(Box::new(move || {
                    tx.send(weak.upgrade().unwrap().slot_busy(3)).unwrap()
                })),
            )
        });
        assert!(
            !rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            "recovery notification cannot run while its old slot is occupied"
        );
        assert_eq!(ex.counters().started.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn c8c_dropped_idle_executor_releases_its_workers() {
        let ex = Executor::new(2);
        let weak = Arc::downgrade(&ex.inner);
        drop(ex);
        let deadline = Instant::now() + Duration::from_secs(2);
        while weak.strong_count() != 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            weak.strong_count(),
            0,
            "dropping an idle discovery pool must not leave permanent worker threads"
        );
    }

    #[test]
    fn c8b_native_parent_must_agree_with_the_selected_snapshot_chain() {
        let (tree, mut map) = table(vec![
            (1, 0, obs(1, 1, "/bin/sh", &["sh"])),
            (10, 1, obs(10, 10, "/u/node", &["node", "/u/codex"])),
            (11, 10, obs(11, 10, "/u/codex", &["codex"])),
        ]);
        assert_eq!(selected_chain(1, 11, &tree, &map), Ok(vec![10, 11]));
        map.get_mut(&10).unwrap().as_mut().unwrap().identity.parent = 99;
        assert!(
            selected_chain(1, 11, &tree, &map).is_err(),
            "unchanged selected receiver cannot hide a reparented intermediary"
        );
    }

    #[test]
    fn c8b_vanished_intermediate_is_not_an_unrelated_dead_descendant() {
        let (tree, mut map) = table(vec![
            (1, 0, obs(1, 1, "/bin/sh", &["sh"])),
            (10, 1, obs(10, 10, "/u/node", &["node", "/u/codex"])),
            (11, 10, obs(11, 10, "/u/codex", &["codex"])),
            (12, 1, obs(12, 12, "/u/tool", &["tool"])),
        ]);
        map.insert(12, Err(Failure::Dead));
        assert_eq!(selected_chain(1, 11, &tree, &map), Ok(vec![10, 11]));
        map.insert(10, Err(Failure::Dead));
        assert!(
            selected_chain(1, 11, &tree, &map).is_err(),
            "a dead intermediary invalidates the selected ancestry"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn c8b_native_surface_root_can_itself_be_the_receiver() {
        let pid = std::process::id();
        let root = cys::process_observation::observe(pid).unwrap();
        let executable = String::from_utf8_lossy(&root.identity.executable).into_owned();
        let pgid = root.identity.process_group;
        let input = DiscoveryInput {
            root_pid: pid,
            recipe: recipe(base_name(&executable)),
            class: ProofClass::FullRequired,
            epochs: Epochs::default(),
            sealed: vec![],
            snapshot: Box::new(Vec::new),
            foreground: Box::new(move || Some(pgid)),
            observe: Box::new(cys::process_observation::observe),
            retain: Box::new(Retained::open),
        };
        let bound = discover(&input)
            .expect("strict root invocation is a receiver even without descendants");
        assert_eq!(bound.agent_identity().0, pid);
    }

    #[test]
    fn c8b_absolute_deadline_is_part_of_start_admission() {
        let at = Instant::now();
        let mut b = StartBudget::new();
        b.open_window(at, Duration::from_millis(BOOT_PROOF_WINDOW_MS));
        assert_eq!(
            b.may_start(at + Duration::from_millis(BOOT_PROOF_WINDOW_MS)),
            Ok(())
        );
        assert!(
            b.may_start(at + Duration::from_millis(BOOT_PROOF_WINDOW_MS + 1))
                .is_err(),
            "expired proof window must refuse a native start even when budget remains"
        );
        assert_eq!(b.starts, 0);
    }

    #[test]
    fn c8b_enqueued_work_is_not_an_actual_native_start() {
        // No worker has entered this queue. Admission is real, but starting a query is not.
        let ex = Executor {
            inner: Arc::new(ExecInner {
                state: Mutex::new(ExecState {
                    queue: VecDeque::new(),
                    busy: HashSet::new(),
                    running: 0,
                    closed: false,
                }),
                cv: Condvar::new(),
                counters: Counters::default(),
                capacity: 1,
            }),
        };
        ex.try_start(10, Box::new(|| {})).unwrap();
        assert!(ex.slot_busy(10));
        assert_eq!(ex.counters().admissions.load(Ordering::Relaxed), 1);
        assert_eq!(
            ex.counters().started.load(Ordering::Relaxed),
            0,
            "queued admission consumes no actual native start"
        );
    }

    #[test]
    fn c8b_executor_admission_does_not_wait_for_the_pool_mutex() {
        let ex = Arc::new(Executor::new(1));
        let lock = ex.inner.state.lock().unwrap();
        let worker = ex.clone();
        let (entered, entering) = std::sync::mpsc::channel();
        let (done, completed) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            entered.send(()).unwrap();
            done.send(worker.try_start(7, Box::new(|| {}))).unwrap();
        });
        entering.recv_timeout(Duration::from_secs(10)).unwrap();
        let result = completed.recv_timeout(Duration::from_secs(2));
        // Always unblock and join before asserting; a defective blocking admission cannot leak a thread.
        drop(lock);
        thread.join().unwrap();
        assert!(
            result.is_ok(),
            "executor admission must refuse rather than wait for a held global mutex"
        );
        assert!(
            result.unwrap().is_err(),
            "contended admission cannot start a query"
        );
        assert_eq!(ex.counters().started.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn c8b_equal_argv_native_agents_are_still_ambiguous() {
        let (tree, map) = table(vec![
            (
                10,
                1,
                obs(10, 10, "/v/claude", &["claude", "--resume", "x"]),
            ),
            (
                11,
                10,
                obs(11, 10, "/v/claude", &["claude", "--resume", "x"]),
            ),
        ]);
        assert!(
            matches!(
                select_receiver(&recipe("claude"), &tree, &map, Some(Some(10))),
                Err(Unknown::Ambiguous(_))
            ),
            "equal argv and ancestry do not make two native agents one invocation"
        );
    }

    #[test]
    fn c8b_runtime_option_operands_are_not_script_slots() {
        let o = obs(
            10,
            10,
            "/u/bin/node",
            &["node", "--require", "/u/bin/claude", "/u/unrelated.js"],
        );
        assert_eq!(
            match_slot(&recipe("claude"), &o),
            None,
            "runtime option operand is not the entrypoint"
        );
    }

    #[test]
    fn c8b_package_helper_is_not_the_adapter_entrypoint() {
        let o = obs(
            10,
            10,
            "/u/bin/node",
            &["node", "/u/@anthropic-ai/claude-code/helper.js"],
        );
        assert_eq!(
            match_slot(&recipe("claude"), &o),
            None,
            "package membership alone is not an invocation recipe"
        );
    }

    #[test]
    fn c8b_custom_unicode_bin_matching_is_total() {
        let o = obs(10, 10, "/u/도구", &["도구"]);
        let matched = std::panic::catch_unwind(|| match_slot(&recipe("도구"), &o));
        assert!(
            matched.is_ok(),
            "custom invocation matching must not panic on UTF-8"
        );
        assert_eq!(
            matched.unwrap(),
            Some(Slot::Native),
            "custom declarations are UTF-8, not byte offsets"
        );
    }

    #[test]
    fn policy_class_is_fixed_from_the_declaration_alone() {
        assert_eq!(
            classify_policy("claude", "claude", Some("claude"), true),
            (ProofClass::FullRequired, None)
        );
        assert_eq!(
            classify_policy("gemini", "agy", Some("agy"), true),
            (ProofClass::FullRequired, None)
        );
        assert_eq!(
            classify_policy("claude", "my-claude-wrapper", Some("claude"), true),
            (
                ProofClass::ObservedAgentWeak,
                Some(WeakRisk::CustomWrapperReceiverIdentity)
            )
        );
        assert_eq!(
            classify_policy("mytool", "mytool", None, true),
            (
                ProofClass::ObservedAgentWeak,
                Some(WeakRisk::DeclaredCustomInvocation)
            )
        );
        assert_eq!(
            classify_policy("claude", "claude", Some("claude"), false),
            (
                ProofClass::ObservedAgentWeak,
                Some(WeakRisk::NoNativeBackend)
            )
        );
    }

    #[test]
    fn strict_slots_match_measured_recipes_and_reject_operands() {
        // Measured 2026-10-16 on live dept-5 seats (read-only ps/lsof).
        let claude = obs(
            1,
            1,
            "/opt/fixture/.local/share/claude/versions/2.1.300",
            &["claude", "--resume", "x"],
        );
        assert_eq!(match_slot(&recipe("claude"), &claude), Some(Slot::Native));
        let codex_native = obs(
            2,
            2,
            "/x/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex",
            &["/x/codex", "--no-daemon"],
        );
        assert_eq!(
            match_slot(&recipe("codex"), &codex_native),
            Some(Slot::Native)
        );
        let codex_shim = obs(
            3,
            2,
            "/opt/homebrew/bin/node",
            &["node", "/opt/fixture/.local/bin/codex", "--no-daemon"],
        );
        assert_eq!(
            match_slot(&recipe("codex"), &codex_shim),
            Some(Slot::Script(1))
        );
        let npm_claude = obs(
            4,
            4,
            "/usr/local/bin/node",
            &[
                "node",
                "--no-warnings",
                "/n/@anthropic-ai/claude-code/cli.js",
            ],
        );
        assert_eq!(
            match_slot(&recipe("claude"), &npm_claude),
            Some(Slot::Script(2))
        );
        let agy = obs(
            5,
            5,
            "/opt/fixture/.local/bin/agy",
            &["/opt/fixture/.local/bin/agy", "--continue"],
        );
        assert_eq!(match_slot(&recipe("agy"), &agy), Some(Slot::Native));
        // An operand or help text naming the agent is not an invocation.
        assert_eq!(
            match_slot(
                &recipe("claude"),
                &obs(6, 6, "/usr/bin/tail", &["tail", "-f", "/u/claude/log"])
            ),
            None
        );
        assert_eq!(
            match_slot(
                &recipe("claude"),
                &obs(7, 7, "/bin/zsh", &["zsh", "-c", "claude --help"])
            ),
            None
        );
        // argv0 "claude" outside the versioned install dir is not enough.
        assert_eq!(
            match_slot(&recipe("claude"), &obs(8, 8, "/tmp/evil", &["claude"])),
            None
        );
    }

    fn table(
        entries: Vec<(u32, u32, ProcessObservation)>,
    ) -> (
        Vec<(u32, u32)>,
        HashMap<u32, Result<ProcessObservation, Failure>>,
    ) {
        let tree = entries.iter().map(|(p, pp, _)| (*p, *pp)).collect();
        let map = entries
            .into_iter()
            .map(|(p, parent, mut o)| {
                o.identity.parent = parent;
                (p, Ok(o))
            })
            .collect();
        (tree, map)
    }

    #[test]
    fn same_invocation_chain_collapses_to_the_innermost_native_process() {
        let (tree, map) = table(vec![
            (
                10,
                1,
                obs(
                    10,
                    10,
                    "/n/node",
                    &["node", "/u/bin/codex", "--no-daemon", "--x"],
                ),
            ),
            (
                11,
                10,
                obs(
                    11,
                    10,
                    "/v/bin/codex",
                    &["/v/bin/codex", "--no-daemon", "--x"],
                ),
            ),
            // tool children in their own groups are not receivers
            (
                12,
                11,
                obs(
                    12,
                    12,
                    "/v/bin/codex-code-mode-host",
                    &["codex-code-mode-host"],
                ),
            ),
        ]);
        assert_eq!(
            select_receiver(&recipe("codex"), &tree, &map, Some(Some(10))),
            Ok(11)
        );
    }

    #[test]
    fn incomparable_matches_unreadable_or_absent_candidates_are_unknown() {
        let (tree, map) = table(vec![
            (
                10,
                1,
                obs(10, 10, "/c/claude/versions/2", &["claude", "--resume", "a"]),
            ),
            (
                11,
                10,
                obs(11, 10, "/c/claude/versions/2", &["claude", "-p", "nested"]),
            ),
        ]);
        assert!(matches!(
            select_receiver(&recipe("claude"), &tree, &map, Some(Some(10))),
            Err(Unknown::Ambiguous(_))
        ));
        // Same pair, but the nested one is outside the foreground group: not a plausible receiver.
        let (tree, map) = table(vec![
            (
                10,
                1,
                obs(10, 10, "/c/claude/versions/2", &["claude", "--resume", "a"]),
            ),
            (
                11,
                10,
                obs(11, 11, "/c/claude/versions/2", &["claude", "-p", "nested"]),
            ),
        ]);
        assert_eq!(
            select_receiver(&recipe("claude"), &tree, &map, Some(Some(10))),
            Ok(10)
        );
        // An unreadable descendant could be a second receiver: refuse to guess.
        let mut m2 = map.clone();
        m2.insert(
            11,
            Err(Failure::Unknown {
                stage: "KERN_PROCARGS2",
                os_error: Some(22),
            }),
        );
        assert!(matches!(
            select_receiver(&recipe("claude"), &tree, &m2, Some(Some(10))),
            Err(Unknown::CandidateUnreadable(_))
        ));
        // A dead descendant is ignored.
        m2.insert(11, Err(Failure::Dead));
        assert_eq!(
            select_receiver(&recipe("claude"), &tree, &m2, Some(Some(10))),
            Ok(10)
        );
        // Unreadable foreground and no candidate.
        assert_eq!(
            select_receiver(&recipe("claude"), &tree, &map, Some(None)),
            Err(Unknown::ForegroundUnreadable)
        );
        let (tree, map) = table(vec![(10, 1, obs(10, 10, "/bin/zsh", &["zsh"]))]);
        assert_eq!(
            select_receiver(&recipe("claude"), &tree, &map, Some(Some(10))),
            Err(Unknown::NoCandidate)
        );
    }

    #[test]
    fn start_budget_keeps_three_starts_for_eligibility_and_never_moves_the_window() {
        let t0 = Instant::now();
        let mut b = StartBudget::new();
        assert_eq!(b.may_start(t0), Ok(()));
        b.note_started(t0);
        // A second settle-time start is refused (reserve for eligibility).
        assert_eq!(
            b.may_start(t0 + Duration::from_millis(300)),
            Err(StartRefusal::BudgetExhausted)
        );
        let eligible = t0 + Duration::from_millis(2_000);
        b.open_window(eligible, Duration::from_millis(BOOT_PROOF_WINDOW_MS));
        b.open_window(
            eligible + Duration::from_secs(5),
            Duration::from_millis(BOOT_PROOF_WINDOW_MS),
        );
        assert_eq!(
            b.deadline,
            Some(eligible + Duration::from_millis(BOOT_PROOF_WINDOW_MS)),
            "window must not move"
        );
        let mut t = eligible;
        for _ in 0..3 {
            assert_eq!(b.may_start(t), Ok(()));
            b.note_started(t);
            let expect = if b.starts < MAX_STARTS_PER_OBLIGATION {
                StartRefusal::TooSoon
            } else {
                StartRefusal::BudgetExhausted // the budget check precedes spacing
            };
            assert_eq!(b.may_start(t + Duration::from_millis(100)), Err(expect));
            t += Duration::from_millis(MIN_START_SPACING_MS);
        }
        assert_eq!(b.starts, MAX_STARTS_PER_OBLIGATION);
        assert_eq!(
            b.may_start(t + Duration::from_secs(1)),
            Err(StartRefusal::BudgetExhausted)
        );
        assert!(!b.expired(eligible + Duration::from_millis(BOOT_PROOF_WINDOW_MS)));
        assert!(b.expired(eligible + Duration::from_millis(BOOT_PROOF_WINDOW_MS + 1)));
    }

    // Admission is nonblocking; startup/worker bookkeeping may transiently own its mutex.
    // Fixtures retry ONLY AdmissionBusy, without consuming a native start or changing a deadline.
    fn start_when_uncontended(ex: &Executor, slot: u64, mut make: impl FnMut() -> Job) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match ex.try_start(slot, make()) {
                Ok(()) => return,
                Err(StartRefusal::AdmissionBusy) if Instant::now() < deadline => {
                    std::thread::yield_now()
                }
                other => panic!("fixture admission did not become available: {other:?}"),
            }
        }
    }

    #[test]
    fn executor_slot_survives_caller_deadline_and_refusals_start_nothing() {
        let ex = Executor::new(2);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let release = Arc::new(Mutex::new(Some(rx)));
        start_when_uncontended(&ex, 7, || {
            let r = release.clone();
            Box::new(move || {
                let rx = r.lock().unwrap().take().unwrap();
                let _ = rx.recv_timeout(Duration::from_secs(5));
            })
        });
        // Past any caller deadline the slot is still held: a second query for the same surface is refused.
        std::thread::sleep(Duration::from_millis(
            DISCOVERY_CALLER_DEADLINE_PROVISIONAL_MS + 50,
        ));
        assert_eq!(
            ex.try_start(7, Box::new(|| {})),
            Err(StartRefusal::SlotBusy)
        );
        assert_eq!(
            ex.counters().started.load(Ordering::Relaxed),
            1,
            "slot-busy consumes no start"
        );
        // Another surface still gets the remaining worker (one bad surface cannot take all capacity).
        let (dtx, drx) = std::sync::mpsc::channel();
        start_when_uncontended(&ex, 8, || {
            let dtx = dtx.clone();
            Box::new(move || dtx.send(()).unwrap())
        });
        drx.recv_timeout(Duration::from_secs(5))
            .expect("other surface progressed");
        tx.send(()).unwrap();
        for _ in 0..100 {
            if !ex.slot_busy(7) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !ex.slot_busy(7),
            "slot released only after the native call returned"
        );
        assert_eq!(ex.counters().slot_busy.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn executor_saturation_refuses_promptly() {
        let ex = Executor::new(1);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Arc::new(Mutex::new(Some(rx)));
        start_when_uncontended(&ex, 1, || {
            let r = rx.clone();
            Box::new(move || {
                let rx = r.lock().unwrap().take().unwrap();
                let _ = rx.recv_timeout(Duration::from_secs(5));
            })
        });
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            ex.try_start(2, Box::new(|| {})),
            Err(StartRefusal::Saturated)
        );
        tx.send(()).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_discovery_binds_an_owned_child_and_checks_identity() {
        use std::process::{Command, Stdio};
        // Root = this test process' owned shell-like parent: spawn `cat` as the "agent".
        let mut child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .env_clear()
            .spawn()
            .unwrap();
        let cpid = child.id();
        let me = std::process::id();
        let input = DiscoveryInput {
            root_pid: me,
            recipe: recipe("cat"),
            class: ProofClass::FullRequired,
            epochs: Epochs::default(),
            sealed: vec![],
            snapshot: Box::new(move || {
                vec![Node {
                    pid: cpid,
                    parent: me,
                }]
            }),
            foreground: Box::new(|| None),
            observe: Box::new(cys::process_observation::observe),
            retain: Box::new(Retained::open),
        };
        // Owned child shares our process group; foreground is irrelevant for this unit (no PTY):
        let mut b = discover(&DiscoveryInput {
            foreground: Box::new(move || Some(unsafe { libc::getpgrp() } as u32)),
            ..input
        })
        .unwrap();
        b.agent_pgid = None; // no terminal in a unit test
        assert_eq!(b.agent.observation().identity.pid, cpid);
        assert_eq!(b.check(&|| None), Ok(()));
        drop(child.stdin.take());
        child.wait().unwrap();
        let loss = b.check(&|| None).unwrap_err();
        assert!(
            matches!(loss, ProofLoss::AgentDead | ProofLoss::Changed),
            "{loss:?}"
        );
    }
    #[test]
    fn c8_executor_first_spawn_failure_returns_the_original_error() {
        let mut attempted = Vec::new();
        let result = Executor::try_new_with_spawn(2, |i, _task| {
            attempted.push(i);
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "fixture spawn zero"))
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("a failed spawn cannot publish an executor"),
        };
        assert_eq!(attempted, vec![0]);
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "fixture spawn zero");
    }

    #[test]
    fn c8_executor_partial_spawn_failure_closes_and_joins_every_prior_worker() {
        // Each prior worker is the independently held one in one case. Joining only
        // the first OR only the last handle must return too early in one of these cases.
        for held in 0..2 {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (loop_done_tx, loop_done_rx) = std::sync::mpsc::channel();
            let (result_tx, result_rx) = std::sync::mpsc::channel();
            let (exit_tx, exit_rx) = std::sync::mpsc::channel();
            let exited = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed_exited = exited.clone();
            let mut releases = Vec::new();
            let mut waits = Vec::new();
            for _ in 0..2 {
                let (tx, rx) = std::sync::mpsc::channel();
                releases.push(tx);
                waits.push(Some(rx));
            }
            let constructor = std::thread::spawn(move || {
                let result = Executor::try_new_with_spawn(3, |i, task| {
                    if i == 2 {
                        let mut entered = vec![
                            entered_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
                            entered_rx.recv_timeout(Duration::from_secs(10)).unwrap(),
                        ];
                        entered.sort_unstable();
                        assert_eq!(entered, vec![0, 1]);
                        return Err(std::io::Error::new(std::io::ErrorKind::Other, "fixture spawn two"));
                    }
                    let entered_tx = entered_tx.clone();
                    let loop_done_tx = loop_done_tx.clone();
                    let wait = waits[i].take().unwrap();
                    let exited = exited.clone();
                    let exit_tx = exit_tx.clone();
                    std::thread::Builder::new().spawn(move || {
                        entered_tx.send(i).unwrap();
                        task();
                        loop_done_tx.send(i).unwrap();
                        wait.recv_timeout(Duration::from_secs(10)).unwrap();
                        exited.fetch_add(1, Ordering::SeqCst);
                        exit_tx.send(i).unwrap();
                    })
                });
                let result = result.map(|_| ()).map_err(|e| (e.kind(), e.to_string()));
                result_tx.send(result).unwrap();
            });
            let first = loop_done_rx.recv_timeout(Duration::from_secs(10));
            let second = if first.is_ok() { loop_done_rx.recv_timeout(Duration::from_secs(10)) }
                else { Err(std::sync::mpsc::RecvTimeoutError::Timeout) };
            if first.is_err() || second.is_err() {
                for release in &releases { let _ = release.send(()); }
                // A broken-close mutant leaves fixture threads in this exact-test process.
                // Do not join the intentionally stuck constructor; process exit terminates them.
                panic!("constructor must close and wake every prior idle worker");
            }
            let mut done = vec![first.unwrap(), second.unwrap()];
            done.sort_unstable();
            assert_eq!(done, vec![0, 1]);
            let other = 1 - held;
            releases[other].send(()).unwrap();
            assert_eq!(exit_rx.recv_timeout(Duration::from_secs(10)).unwrap(), other,
                "the other worker actually exited while the selected worker remains held");
            let early_result = result_rx.recv_timeout(Duration::from_secs(2));
            // Release the selected worker before reporting a discriminating assertion failure.
            releases[held].send(()).unwrap();
            let result = match early_result {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(other) => panic!("constructor disconnected before result: {other:?}"),
            };
            let returned_early = result.is_some();
            let result = result.unwrap_or_else(|| result_rx.recv_timeout(Duration::from_secs(10)).unwrap());
            constructor.join().unwrap();
            assert_eq!(exit_rx.recv_timeout(Duration::from_secs(10)).unwrap(), held);
            assert!(!returned_early,
                "the constructor returned before its prior threads were released (held worker {held})");
            assert_eq!(result, Err((std::io::ErrorKind::Other, "fixture spawn two".into())));
            assert_eq!(observed_exited.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn c8_executor_factory_unwind_still_cleans_prior_idle_worker() {
        let exited = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_exited = exited.clone();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = Executor::try_new_with_spawn(2, |i, task| {
                if i == 1 {
                    panic!("deliberate spawn factory panic");
                }
                let exited = exited.clone();
                std::thread::Builder::new().spawn(move || {
                    task();
                    exited.fetch_add(1, Ordering::SeqCst);
                })
            });
        }))
        .expect_err("the exact injected factory panic must propagate");
        assert_eq!(panic.downcast_ref::<&str>().copied(), Some("deliberate spawn factory panic"));
        assert_eq!(observed_exited.load(Ordering::SeqCst), 1, "factory unwind must join its prior idle worker");
    }

    #[test]
    fn c8_executor_idle_worker_exit_panic_does_not_replace_spawn_error() {
        let caught = std::panic::catch_unwind(|| {
            Executor::try_new_with_spawn(2, |i, task| {
                if i == 1 {
                    return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "original spawn failure"));
                }
                std::thread::Builder::new().spawn(move || {
                    task();
                    panic!("deliberate idle worker exit panic");
                })
            })
        });
        let result = match caught {
            Ok(result) => result,
            Err(_) => panic!("worker exit panic replaced the constructor's io::Error"),
        };
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("failed constructor published a pool"),
        };
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "original spawn failure");
    }

    #[test]
    fn c8_executor_success_disarms_cleanup_and_preserves_worker_names() {
        let ex = Executor::try_new(2).expect("bounded real spawn fixture");
        let (tx, rx) = std::sync::mpsc::channel();
        start_when_uncontended(&ex, 61, || {
            let tx = tx.clone();
            Box::new(move || tx.send(std::thread::current().name().map(str::to_owned)).unwrap())
        });
        let name = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert!(name == "cysd-discovery-0" || name == "cysd-discovery-1");
        assert_eq!(ex.counters().started.load(Ordering::Relaxed), 1);
        let weak = Arc::downgrade(&ex.inner);
        drop(ex);
        let deadline = Instant::now() + Duration::from_secs(10);
        while weak.strong_count() != 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn c8_executor_normal_drop_does_not_join_a_blocked_native_job() {
        let ex = Executor::try_new(1).unwrap();
        let weak = Arc::downgrade(&ex.inner);
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release = Arc::new(Mutex::new(Some(release_rx)));
        start_when_uncontended(&ex, 77, || {
            let entered_tx = entered_tx.clone();
            let release = release.clone();
            Box::new(move || {
                entered_tx.send(()).unwrap();
                // Bounded fixture wait; this is a controlled blocked job, not native discovery.
                release.lock().unwrap().take().unwrap().recv_timeout(Duration::from_secs(10)).unwrap();
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(ex);
            dropped_tx.send(()).unwrap();
        });
        let returned_while_blocked = dropped_rx.recv_timeout(Duration::from_secs(2));
        let retained_while_blocked = weak.strong_count() > 0;
        // Always release before asserting, including a mutant that incorrectly joins on Drop.
        release_tx.send(()).unwrap();
        dropper.join().unwrap();
        assert!(returned_while_blocked.is_ok(), "ordinary Drop waited for blocked work");
        assert!(retained_while_blocked, "running worker lost its inner before return");
        let deadline = Instant::now() + Duration::from_secs(10);
        while weak.strong_count() != 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn c8_executor_zero_workers_preserves_empty_pool_behavior() {
        let ex = Executor::try_new_with_spawn(0, |_, _| panic!("zero workers must not spawn")).unwrap();
        assert_eq!(ex.try_start(1, Box::new(|| panic!("no worker"))), Err(StartRefusal::Saturated));
    }

}
