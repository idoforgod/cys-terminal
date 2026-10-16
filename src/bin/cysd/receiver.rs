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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofClass {
    FullRequired,
    ObservedAgentWeak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationOrigin {
    LaunchAgent,
    ClaimRole,
    Reclaim,
    HandObserved,
    Restored,
}

impl RegistrationOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LaunchAgent => "launch-agent",
            Self::ClaimRole => "claim-role",
            Self::Reclaim => "reclaim",
            Self::HandObserved => "hand-observed",
            Self::Restored => "restored",
        }
    }
}

/// The named residual risk carried by every weak receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// Base name of the configured binary (`agent_candidates_from_json` semantics).
    pub bin: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverPolicy {
    pub policy_epoch: u64,
    pub adapter: String,
    pub recipe: Recipe,
    pub origin: RegistrationOrigin,
    pub class: ProofClass,
    pub weak_risk: Option<WeakRisk>,
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
        name.len() > suf.len() && name[name.len() - suf.len()..].eq_ignore_ascii_case(suf)
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
        // First non-option argument after argv0 is the script slot.
        let idx = obs
            .invocation
            .argv
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, a)| !a.starts_with(b"-"))?
            .0;
        let script = lossy(&obs.invocation.argv[idx]);
        let sbase = base_name(&script);
        let stem = sbase
            .strip_suffix(".js")
            .or_else(|| sbase.strip_suffix(".mjs"))
            .or_else(|| sbase.strip_suffix(".cjs"))
            .unwrap_or(sbase);
        let pkg_cli = format!("{}-cli", recipe.bin);
        let pkg_code = format!("{}-code", recipe.bin);
        let pkg_hit = sbase.ends_with(".js")
            && script
                .split(['/', '\\'])
                .any(|seg| seg == pkg_cli || seg == pkg_code);
        if bin_eq(stem, &recipe.bin) || pkg_hit {
            return Some(Slot::Script(idx));
        }
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
/// identical adapter arguments are the same invocation and collapse to the innermost; anything
/// else (incomparable matches, an unreadable matching process, no candidate) is Unknown.
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
    let is_ancestor = |anc: u32, mut p: u32| {
        for _ in 0..4096 {
            match parent.get(&p) {
                Some(&q) if q == anc => return true,
                Some(&q) => p = q,
                None => return false,
            }
        }
        false
    };
    for w in matches.windows(2) {
        let (a, sa) = w[0];
        let (b, sb) = w[1];
        let same_chain = is_ancestor(a, b);
        let oa = observed[&a].as_ref().unwrap();
        let ob = observed[&b].as_ref().unwrap();
        if !same_chain || adapter_args(oa, sa) != adapter_args(ob, sb) {
            return Err(Unknown::Ambiguous(format!("{a},{b}")));
        }
    }
    Ok(matches.last().unwrap().0)
}

/// Epochs that key a binding (design 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Epochs {
    pub launch: u64,
    pub conversation: u64,
    pub policy: u64,
    pub binding: u64,
}

/// A daemon-owned native receiver binding.
pub struct Binding {
    pub epochs: Epochs,
    pub class: ProofClass,
    pub root: Retained,
    pub agent: Retained,
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

/// Run one discovery: snapshot once, read only the root and its descendants, select, retain the
/// selection, and RE-READ the root identity before publishing (design 4.1).
pub fn discover(input: &DiscoveryInput) -> Result<Binding, Unknown> {
    if !native_backend_available() {
        return Err(Unknown::Unsupported);
    }
    let root = (input.retain)(input.root_pid).map_err(|f| match f {
        Failure::Dead => Unknown::RootDead,
        f => Unknown::RootUnreadable(failure_reason(&f)),
    })?;
    let nodes = (input.snapshot)();
    let tree = descendants(&nodes, input.root_pid);
    let mut observed = HashMap::new();
    for (pid, _) in &tree {
        observed.insert(*pid, (input.observe)(*pid));
    }
    let fg = if cfg!(unix) {
        Some((input.foreground)())
    } else {
        None
    };
    let pid = select_receiver(&input.recipe, &tree, &observed, fg)?;
    let agent = (input.retain)(pid).map_err(|f| match f {
        Failure::Dead => Unknown::CandidateVanished,
        f => Unknown::CandidateUnreadable(format!("{pid}:{}", failure_reason(&f))),
    })?;
    // The retained object must be the process that was selected (no pid reuse in between).
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
    match root.recheck() {
        Ok(()) => {}
        Err(Failure::Dead) => return Err(Unknown::RootDead),
        Err(_) => return Err(Unknown::RootChanged),
    }
    let agent_pgid = cfg!(unix).then_some(id.process_group);
    Ok(Binding {
        epochs: input.epochs,
        class: input.class,
        root,
        agent,
        agent_pgid,
    })
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
}

#[derive(Default, Debug)]
pub struct Counters {
    pub admissions: AtomicU64,
    pub started: AtomicU64,
    pub slot_busy: AtomicU64,
    pub saturated: AtomicU64,
    pub caller_deadlines: AtomicU64,
    pub late_completions: AtomicU64,
    pub late_after_window: AtomicU64,
}

type Job = Box<dyn FnOnce() + Send>;

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

struct ExecState {
    queue: VecDeque<(u64, Job)>,
    busy: HashSet<u64>,
    running: usize,
}

impl Executor {
    pub fn new(workers: usize) -> Executor {
        let inner = Arc::new(ExecInner {
            state: Mutex::new(ExecState {
                queue: VecDeque::new(),
                busy: HashSet::new(),
                running: 0,
            }),
            cv: Condvar::new(),
            counters: Counters::default(),
            capacity: workers,
        });
        for i in 0..workers {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name(format!("cysd-discovery-{i}"))
                .spawn(move || loop {
                    let (slot, job) = {
                        let mut st = inner.state.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(j) = st.queue.pop_front() {
                                st.running += 1;
                                break j;
                            }
                            st = inner.cv.wait(st).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    // A panicking job must not kill the worker or leak its slot.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    let mut st = inner.state.lock().unwrap_or_else(|e| e.into_inner());
                    st.running -= 1;
                    st.busy.remove(&slot);
                })
                .expect("spawn discovery worker");
        }
        Executor { inner }
    }

    pub fn counters(&self) -> &Counters {
        &self.inner.counters
    }

    /// Admit a native query for `slot` (a surface instance). The slot stays occupied until the
    /// job RETURNS — a caller deadline never frees it and never starts a second query (S3-2).
    pub fn try_start(&self, slot: u64, job: Job) -> Result<(), StartRefusal> {
        self.inner
            .counters
            .admissions
            .fetch_add(1, Ordering::Relaxed);
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
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
        st.queue.push_back((slot, job));
        self.inner.counters.started.fetch_add(1, Ordering::Relaxed);
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
        if self.starts >= MAX_STARTS_PER_OBLIGATION {
            return Err(StartRefusal::BudgetExhausted);
        }
        if let Some(t) = self.last_start {
            if now.duration_since(t) < Duration::from_millis(MIN_START_SPACING_MS) {
                return Err(StartRefusal::TooSoon);
            }
        }
        let before_eligible = self.eligible_at.map_or(true, |e| now < e);
        if before_eligible && self.starts >= 1 {
            return Err(StartRefusal::BudgetExhausted); // keep three starts for eligibility
        }
        Ok(())
    }

    /// Record an ACTUAL start (call only when the executor admitted the job).
    pub fn note_started(&mut self, now: Instant) {
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
        Recipe { bin: bin.into() }
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
        let map = entries.into_iter().map(|(p, _, o)| (p, Ok(o))).collect();
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

    #[test]
    fn executor_slot_survives_caller_deadline_and_refusals_start_nothing() {
        let ex = Executor::new(2);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let release = Arc::new(Mutex::new(Some(rx)));
        let r = release.clone();
        ex.try_start(
            7,
            Box::new(move || {
                let rx = r.lock().unwrap().take().unwrap();
                let _ = rx.recv_timeout(Duration::from_secs(5));
            }),
        )
        .unwrap();
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
        ex.try_start(8, Box::new(move || dtx.send(()).unwrap()))
            .unwrap();
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
        let r = rx.clone();
        ex.try_start(
            1,
            Box::new(move || {
                let rx = r.lock().unwrap().take().unwrap();
                let _ = rx.recv_timeout(Duration::from_secs(5));
            }),
        )
        .unwrap();
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
}
