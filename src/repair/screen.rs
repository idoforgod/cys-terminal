//! The protection SCREEN (C4 — RULINGS AT1, AT2, AT9, AY1–AY5, BA1–BA6, BC1, BC3–BC5, BC7, BD1–BD3,
//! BD5, BG1, BH1–BH3, BH5, BK1, BL1; design D:941 "넷을 구현하는 파일·등록·시험의 목록을 본체에 넣고, 수리·
//! 합치기·등록 해제의 모든 후보에 대해 '이 후보를 적용한 뒤에도 넷의 시험이 통과하는가'를 확인한다").
//!
//! Track T5b. The ENTRY POINT, its signature and the public fields of [`ScreenReport`] are the seam
//! of track T6 (called at apply) and are fixed here; they change only through a contract change.
//! `submit` (T5b) calls [`screen_protections`] as part of step 3 of the approval procedure; the
//! apply (T6) first asks [`recorded_verdict_holds`] and runs the screen only when the recorded
//! verdict does not hold for the inputs and the binary at hand.

use super::types::{RepairError, RepairId, TargetState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Version of the screen's tables and rules; part of [`screen_build_id`] and of the recorded verdict.
pub const SCREEN_SCHEMA: u32 = 1;

/// What the screen is asked about (RULINGS AV3 / U2): every declared FILE target of the candidate
/// with its candidate STATE, and the folder that holds the candidate's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCandidate {
    /// `<files>/<rel>` holds the bytes of each target whose state is [`TargetState::File`] — the
    /// repair's working copy (`paths::staging_dir`) at `submit`, the vault slot at apply.
    pub files: PathBuf,
    /// Every declared file target with its candidate state, as `repair::candidate_states` returns
    /// them: `Absent` = the candidate deletes that file, `Link` = the candidate makes it that link.
    pub targets: Vec<(String, TargetState)>,
}

/// The screen's verdict (AY5): accepted, accepted with notes (lane findings / not-measurable probes
/// of devices the candidate does not touch), or refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenVerdict {
    Accepted,
    AcceptedWithNotes,
    Refused,
}

/// A device's answer to one probe (AY1 / BD5): DENY (exit 2, or exit 0 with a deny payload), ALLOW,
/// or ERROR (a crash, a timeout, anything else). For LANE, CANDIDATE, CLEAN and the attribution run
/// an ERROR counts as "not denied", never as DENY.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Answer {
    Deny,
    Allow,
    Error,
}

/// What one probe in one cell decided (AY1 (1)–(5), BD1, BC5, BG1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeDecision {
    /// RELEASED allows it there — the probe binds nothing in this cell.
    Unbound,
    /// It binds and the candidate keeps it denied.
    Held,
    /// It binds and the candidate lets it through (CLEAN, the lane comparison or the attribution run).
    Refused,
    /// It binds, neither LANE nor CANDIDATE denies, and the attribution run does not deny either:
    /// the cause lies in the lane — a note and one intake row; the candidate proceeds.
    LaneFinding,
    /// RELEASED errors there. Refuses at `submit` only when the candidate touches an input file of
    /// that device; at apply it always declines (C4 AY5).
    NotMeasurable,
}

/// One probe of one class in one cell, or one judged (layout, scenario, run, target) of invariant
/// I3, or one region of invariant I4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
    /// The protection test: `G`, `R`, `C`, `I3` or `I4`.
    pub test: String,
    /// The class (`G-1` … `G-24`, `R-1` … `R-16`, `C-1` … `C-25`), the I3 target or the I4 region.
    pub class: String,
    /// The cell (`E-1` … `E-21`), or for I3 `<layout>/<scenario>/run<n>`, for I4 the region name.
    pub cell: String,
    /// The protection concerned, in the owner's words (Korean).
    pub protection: String,
    pub released: Answer,
    pub clean: Option<Answer>,
    pub lane: Option<Answer>,
    pub candidate: Option<Answer>,
    pub attribution: Option<Answer>,
    pub decision: ProbeDecision,
    /// Why (one line).
    pub detail: String,
}

/// One compared item of the scoped after-run comparison (C4 replay rules (c), BK1 / BL1): a live
/// listed file of the lane or the hook sections of a real settings file, with its state and its
/// change stamp before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveDifference {
    /// The compared item (an absolute path; for a settings file `<path>#hooks` / `<path>#statusLine`).
    pub item: String,
    pub before: String,
    pub after: String,
}

/// The scoped after-run comparison of a screen run (BK1, BL1). The library reports raw differences;
/// the daemon drops those its OWN recorded operations ON THE LIVE INSTALLATION explain (an applied
/// repair, a revert, an update install, a registrar run on the live installation — never a run of a
/// replay, a rehearsal or the screen; DEFERRED #60).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterRun {
    /// Differences seen across the first set of runs.
    pub first: Vec<LiveDifference>,
    /// When `first` was not empty: the items written AGAIN (state or stamp changed, same bytes
    /// included — BL1) during the one repeat on a fresh scratch root.
    pub rewritten_in_repeat: Vec<LiveDifference>,
    /// Processes the screen started that were still alive after its runs (killed then; must be empty).
    pub leftover_processes: Vec<u32>,
    /// RULINGS CZ6: `first` was not empty and the repeat ran out of time or did not complete — it
    /// proves nothing (never "not written again").
    #[serde(default)]
    pub repeat_incomplete: bool,
}

/// What the screen found (C4 AY5; read by `submit` and by T6's apply).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenReport {
    pub schema: u32,
    pub verdict: ScreenVerdict,
    /// One sentence per refusing ground, each naming the protection concerned.
    pub refusals: Vec<String>,
    /// Notes recorded with an accepted verdict ("보호장치 시험 N건 …", not-measurable devices the
    /// candidate does not touch, an after-run difference that was not written again).
    pub notes: Vec<String>,
    /// One line per lane finding — each becomes one intake row naming the protection.
    pub lane_findings: Vec<String>,
    /// Every probe that was NOT MEASURABLE (at apply each of them declines — AY5).
    pub not_measurable: Vec<String>,
    /// Every probe run (bound or not) with the answers of each variant.
    pub probes: Vec<ProbeResult>,
    /// [`input_fingerprint`] of the candidate in this lane (BC7).
    pub input_fingerprint: String,
    /// [`screen_build_id`] of the binary that ran the screen (BD5).
    pub build_id: String,
    /// Wall time of the whole screen, milliseconds (measured, never estimated).
    pub duration_ms: u64,
    pub after_run: AfterRun,
}

impl ScreenReport {
    /// At apply (C4 AY5): a refusal, or any NOT-MEASURABLE probe, declines the apply.
    pub fn declines_apply(&self) -> bool {
        self.verdict == ScreenVerdict::Refused || !self.not_measurable.is_empty()
    }
}

/// The verdict recorded at `submit` (AY5) — `repairs/R-<n>/screen.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedVerdict {
    pub schema: u32,
    pub verdict: ScreenVerdict,
    pub input_fingerprint: String,
    pub build_id: String,
    pub notes: Vec<String>,
    pub at: f64,
}

/// Runs the protection screen for `candidate` in the lane whose live pack root is `lane`, within
/// `deadline` (wall time for the whole screen; exceeded → `Err(CheckTimeout)` — callers fail closed).
/// RELEASED = the copies embedded in this binary (RULINGS CM1). The scoped after-run comparison is
/// judged with no difference explained (the library knows no daemon operation — a caller that does
/// re-judges with [`judge_after_run`]).
pub fn screen_protections(
    candidate: &ScreenCandidate,
    lane: &Path,
    deadline: Duration,
) -> Result<ScreenReport, RepairError> {
    if !cfg!(unix) {
        // RULINGS AT6: no screen runs on Windows in release 1 (`submit` answers pending there).
        return Err(RepairError::PlatformPending("screen::screen_protections"));
    }
    let cfg = ScreenConfig::production();
    let mut report = screen_with(&cfg, candidate, lane, deadline)?;
    judge_after_run(&mut report, &|_| false);
    Ok(report)
}

/// The fingerprint of the screen's input STATES as they will be after the apply (BC7): every file
/// of the generated per-test inputs (G, R, C, I3), the candidate's state for a target, else the
/// lane's live state — `repair::fingerprint` over those (rel, state) lines.
pub fn input_fingerprint(candidate: &ScreenCandidate, lane: &Path) -> Result<String, RepairError> {
    let rels = all_input_rels()?;
    let mut lines = Vec::with_capacity(rels.len());
    for rel in rels {
        let st = match candidate.targets.iter().find(|(r, _)| *r == rel) {
            Some((_, s)) => s.clone(),
            None => TargetState::read(&lane.join(&rel))?,
        };
        lines.push((rel, st));
    }
    Ok(super::types::fingerprint(&lines))
}

/// RULINGS CZ5: [`input_fingerprint`]'s lines over the inputs as the screen READ them.
fn fingerprint_of_read(v: &Variants) -> String {
    let lines: Vec<(String, TargetState)> = v
        .inputs_read
        .iter()
        .map(|(r, c)| (r.clone(), c.state()))
        .collect();
    super::types::fingerprint(&lines)
}

/// RULINGS CZ5 — the verdict to record at `submit` (AY5), or `None`: a refusal is never recorded, and
/// neither is a verdict whose tested inputs the live lane no longer matches (the lane changed during
/// the screen — the apply then screens again, as for any verdict that does not hold).
pub fn recordable_verdict(
    report: &ScreenReport,
    candidate: &ScreenCandidate,
    lane: &Path,
) -> Result<Option<RecordedVerdict>, RepairError> {
    if report.verdict == ScreenVerdict::Refused
        || report.input_fingerprint != input_fingerprint(candidate, lane)?
    {
        return Ok(None);
    }
    Ok(Some(RecordedVerdict {
        schema: SCREEN_SCHEMA,
        verdict: report.verdict,
        input_fingerprint: report.input_fingerprint.clone(),
        build_id: report.build_id.clone(),
        notes: report.notes.clone(),
        at: super::ledger::now_epoch(),
    }))
}

/// The screen's build id (BD5): a recorded verdict with another build id is never reused.
pub fn screen_build_id() -> String {
    // the embedded pack's hash ties the id to the RELEASED copies the screen compares against (a
    // development build whose git id is "unknown" still gets an id of its own)
    let pack = crate::pack::embedded_pack_hash();
    format!(
        "{}+{}+screen{}+{}",
        crate::pack::build_id(),
        env!("CARGO_PKG_VERSION"),
        SCREEN_SCHEMA,
        pack.get(..16).unwrap_or(&pack)
    )
}

/// `<root>/R-<n>/screen.json` — the recorded verdict of a repair (root = the ledger root).
pub fn verdict_path_in(root: &Path, id: &RepairId) -> PathBuf {
    super::ledger::repair_dir_in(root, id).join("screen.json")
}

/// Writes the recorded verdict atomically (tmp + rename) and durably (RULINGS CM2: written by the
/// lane's daemon only).
pub fn write_verdict_in(
    root: &Path,
    id: &RepairId,
    v: &RecordedVerdict,
) -> Result<(), RepairError> {
    let p = verdict_path_in(root, id);
    let mut v = v.clone();
    v.schema = SCREEN_SCHEMA;
    crate::atomic_write_json(&p, &v)
        .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
    let f = replay::open_nofollow_nonblock(&p)
        .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
    super::ledger::full_flush_file(&f)
        .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
    super::ledger::full_flush_dir(&super::ledger::repair_dir_in(root, id))
}

/// Reads the recorded verdict; `Ok(None)` when none was recorded; a file that cannot be read or
/// has a higher schema → `Err(Corrupt)` (never a verdict that holds).
pub fn read_verdict_in(root: &Path, id: &RepairId) -> Result<Option<RecordedVerdict>, RepairError> {
    let p = verdict_path_in(root, id);
    // one no-follow, non-blocking read (a FIFO in its place cannot hang the apply — RULINGS DF5)
    let bytes = match replay::read_stamped(&p) {
        Ok((LaidSource::Absent, _)) => return Ok(None),
        Ok((LaidSource::Bytes { bytes, .. }, _)) => bytes,
        Ok((LaidSource::Link(_), _)) => {
            return Err(RepairError::Corrupt(format!("{}: a link, not a verdict", p.display())))
        }
        Err(e) => return Err(RepairError::Corrupt(format!("{}: {e:?}", p.display()))),
    };
    let v: RecordedVerdict = serde_json::from_slice(&bytes)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))?;
    if v.schema > SCREEN_SCHEMA {
        return Err(RepairError::Corrupt(format!(
            "{}: unknown schema {}",
            p.display(),
            v.schema
        )));
    }
    Ok(Some(v))
}

/// AY5 at apply: the recorded verdict holds — nothing runs — when it is not a refusal, its build id
/// is this binary's [`screen_build_id`] and its fingerprint equals [`input_fingerprint`] of the
/// candidate in `lane`.
pub fn recorded_verdict_holds(
    recorded: &RecordedVerdict,
    candidate: &ScreenCandidate,
    lane: &Path,
) -> Result<bool, RepairError> {
    if recorded.verdict == ScreenVerdict::Refused
        || recorded.schema != SCREEN_SCHEMA
        || recorded.build_id != screen_build_id()
    {
        return Ok(false);
    }
    Ok(recorded.input_fingerprint == input_fingerprint(candidate, lane)?)
}

/// The scoped after-run comparison judged (BK1, BL1): differences `explained` (an operation the
/// daemon performed and recorded ON THE LIVE INSTALLATION — never a run of a replay, a rehearsal or
/// the screen, DEFERRED #60) are dropped; an item written again during the repeat → REFUSED; a
/// difference not written again → a note (the first difference stays on the intake row).
pub fn judge_after_run(report: &mut ScreenReport, explained: &dyn Fn(&LiveDifference) -> bool) {
    let first: Vec<&LiveDifference> = report
        .after_run
        .first
        .iter()
        .filter(|d| !explained(d))
        .collect();
    if first.is_empty() {
        return;
    }
    if report.after_run.repeat_incomplete {
        // RULINGS CZ6: an incomplete repeat is a failure (§8-1 — not measured is not passed)
        let items: Vec<&str> = first.iter().map(|d| d.item.as_str()).take(5).collect();
        let line = format!(
            "예행 동안 실제 환경의 항목이 바뀌었고({}) 다시 돌린 예행이 시한 안에 끝나지 않아 다시 썼는지 잴 수 없습니다 — 잴 수 없는 것은 통과가 아니므로 거부합니다",
            items.join(", ")
        );
        report.not_measurable.push(line.clone());
        report.refusals.push(line);
        report.verdict = ScreenVerdict::Refused;
        return;
    }
    let again: Vec<&LiveDifference> = report
        .after_run
        .rewritten_in_repeat
        .iter()
        .filter(|d| !explained(d) && first.iter().any(|f| f.item == d.item))
        .collect();
    if !again.is_empty() {
        let items: Vec<&str> = again.iter().map(|d| d.item.as_str()).take(5).collect();
        report.refusals.push(format!(
            "예행이 실제 환경의 항목을 다시 썼습니다({}) — 보호장치 시험 밖으로 새는 후보로 보고 거부합니다",
            items.join(", ")
        ));
        report.verdict = ScreenVerdict::Refused;
    } else {
        let items: Vec<&str> = first.iter().map(|d| d.item.as_str()).take(5).collect();
        let line = format!(
            "예행 동안 실제 환경의 항목이 바뀌었으나 다시 돌렸을 때는 바뀌지 않았습니다({}) — 접수 줄에 남깁니다",
            items.join(", ")
        );
        report.notes.push(line.clone());
        report.lane_findings.push(line);
        if report.verdict == ScreenVerdict::Accepted {
            report.verdict = ScreenVerdict::AcceptedWithNotes;
        }
    }
}

/// RULINGS DE1: the refusal the scoped after-run comparison gave this report — an item written again
/// in the repeat, an incomplete repeat, a process left behind — or `None`. The caller files ONE intake
/// row and the owner's one line for it (BK1), nothing more.
pub fn after_run_refusal(report: &ScreenReport) -> Option<String> {
    if !report.after_run.leftover_processes.is_empty() {
        return Some(format!(
            "예행이 끝난 뒤에도 남은 프로세스가 있었습니다(pid {:?})",
            report.after_run.leftover_processes
        ));
    }
    let mut only = report.clone();
    only.verdict = ScreenVerdict::Accepted;
    only.refusals.clear();
    only.notes.clear();
    only.lane_findings.clear();
    only.not_measurable.clear();
    judge_after_run(&mut only, &|_| false);
    (only.verdict == ScreenVerdict::Refused).then(|| only.refusals.join(" / "))
}

// ═════════════════════════════════════════ the engine ═════════════════════════════════════════

use super::replay::{self, LaidSource, ScratchRoot, ScratchSocket};
use super::screen_corpus::{
    self as corpus, Cell, Class, Device, Probe, Setup, BACKSTOP_CLASSES, BACKSTOP_MARKER, CELLS,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The content of a test's input files in one variant (rel → content).
type Files = BTreeMap<String, LaidSource>;

/// Where the released copies come from (the binary's own in production; injected by tests).
pub(crate) type ReleasedFn = Arc<dyn Fn(&str) -> Option<LaidSource> + Send + Sync>;

/// How a screen runs. Production values come from [`ScreenConfig::production`]; tests narrow the
/// corpus and inject the released source and the scratch base.
#[derive(Clone)]
pub(crate) struct ScreenConfig {
    pub scratch_base: PathBuf,
    pub interpreter: Option<PathBuf>,
    /// The real HOME whose settings files the after-run comparison watches.
    pub real_home: PathBuf,
    pub released: ReleasedFn,
    /// Only these classes (ids) — `None` = every class (production).
    pub classes: Option<Vec<&'static str>>,
    /// Only these cells (ids) — `None` = every cell (production).
    pub cells: Option<Vec<&'static str>>,
    pub run_i3: bool,
    pub threads: usize,
    /// Cap of one child's wall time.
    pub child_cap: Duration,
    /// Test seam (RULINGS CZ5): runs right after the inputs were read, before any probe.
    pub after_inputs_read: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Test seam (RULINGS CZ6): runs with 1 right after the first "before" snapshot and with 2 right
    /// after the repeat's "before" snapshot.
    pub between_snapshots: Option<Arc<dyn Fn(u32) + Send + Sync>>,
    /// Test seam (RULINGS EL1): the first execution of the repeat fails to start.
    pub fail_first_spawn_in_repeat: bool,
    /// Test seam (RULINGS CZ6 / EZ1): the repeat finds its time used up (no sleeping in the tests).
    pub repeat_out_of_time: bool,
}

impl ScreenConfig {
    pub(crate) fn production() -> Self {
        Self {
            scratch_base: super::rehearsal::scratch_base(),
            interpreter: replay::bundled_interpreter(),
            real_home: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            released: Arc::new(|rel: &str| super::rehearsal::released_copy(rel)),
            classes: None,
            cells: None,
            run_i3: true,
            threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(2)
                .clamp(1, 8),
            child_cap: Duration::from_secs(30),
            after_inputs_read: None,
            between_snapshots: None,
            fail_first_spawn_in_repeat: false,
            repeat_out_of_time: false,
        }
    }

    fn class_on(&self, id: &str) -> bool {
        self.classes.as_ref().map_or(true, |v| v.contains(&id))
    }

    fn cell_on(&self, id: &str) -> bool {
        self.cells.as_ref().map_or(true, |v| v.contains(&id))
    }
}

/// The protections, in the owner's words (design §4-12).
fn protection_of(test: &str) -> &'static str {
    match test {
        "G" | "I4" => "외부 발송·영구 삭제 전 주인 승인(hooks/guard.sh)",
        "R" => "리뷰어는 검토 대상을 못 고침(hooks/role-capability-gate.sh)",
        "C" => "CSO 파일 쓰기 제한(hooks/role-capability-gate.sh)",
        _ => "리뷰어·CSO 쓰기 제한 관문의 등록(javis_preflight.py C28)",
    }
}

/// Every input file of the generated per-test inputs (AU1, BM5) — exactly G, R, C, I3.
fn inputs_by_test() -> Result<BTreeMap<&'static str, Vec<String>>, RepairError> {
    let mut m = BTreeMap::new();
    for (t, files) in super::boot_critical::protection_inputs() {
        m.insert(*t, files.iter().map(|f| f.to_string()).collect::<Vec<_>>());
    }
    for t in ["G", "R", "C", "I3"] {
        if m.get(t).map_or(true, |v: &Vec<String>| v.is_empty()) {
            return Err(RepairError::Precondition(format!(
                "보호장치 시험 {t} 의 입력 목록이 이 빌드에 없습니다 — 심사를 할 수 없습니다"
            )));
        }
    }
    if m.len() != 4 {
        return Err(RepairError::Precondition(format!(
            "보호장치 시험 입력이 G·R·C·I3 넷이 아닙니다: {:?}",
            m.keys().collect::<Vec<_>>()
        )));
    }
    Ok(m)
}

fn all_input_rels() -> Result<Vec<String>, RepairError> {
    let mut s = BTreeSet::new();
    for v in inputs_by_test()?.values() {
        s.extend(v.iter().cloned());
    }
    Ok(s.into_iter().collect())
}

/// A file the I3 layout gives every variant from RELEASED (C4: "THE SAME FOR ALL: the released
/// directives/CSO_DIRECTIVE.md").
const SAME_FOR_ALL: &[&str] = &["directives/CSO_DIRECTIVE.md"];

/// The five contents of every input file (AY1, AV3, BD1, BC5).
#[derive(Debug, Clone)]
struct Variants {
    released: Files,
    lane: Files,
    candidate: Files,
    clean: Files,
    attribution: Files,
    /// The candidate's targets that are inputs, and whether each is byte-equal to RELEASED (BG1).
    touched: BTreeMap<String, bool>,
    /// RULINGS CZ5: every input as the screen READ it — the candidate's content for a target, else
    /// the lane's live content — the immutable inputs the recorded fingerprint is computed from.
    inputs_read: Files,
}

fn group_key(f: &Files) -> String {
    let lines: Vec<(String, TargetState)> = f.iter().map(|(r, s)| (r.clone(), s.state())).collect();
    super::types::fingerprint(&lines)
}

fn plain_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !Path::new(rel).is_absolute()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn build_variants(
    cfg: &ScreenConfig,
    cand: &ScreenCandidate,
    lane: &Path,
    rels: &[String],
) -> Result<Variants, RepairError> {
    // the candidate's own contents, checked against the states it declares
    let mut over: BTreeMap<String, LaidSource> = BTreeMap::new();
    for (rel, st) in &cand.targets {
        if !plain_rel(rel) {
            return Err(RepairError::Precondition(format!(
                "{rel}: 팩 안의 상대 경로가 아닙니다"
            )));
        }
        let content = match st {
            TargetState::Absent => LaidSource::Absent,
            TargetState::Link { target } => LaidSource::Link(target.clone()),
            TargetState::File { .. } => {
                let c = LaidSource::read(&cand.files.join(rel))?;
                if c.state() != *st {
                    return Err(RepairError::Precondition(format!(
                        "{rel}: 후보의 바이트가 제출된 상태와 다릅니다 — 심사하지 않습니다"
                    )));
                }
                c
            }
        };
        over.insert(rel.clone(), content);
    }
    let mut v = Variants {
        released: Files::new(),
        lane: Files::new(),
        candidate: Files::new(),
        clean: Files::new(),
        attribution: Files::new(),
        touched: BTreeMap::new(),
        inputs_read: Files::new(),
    };
    for rel in rels {
        let released = (cfg.released)(rel).unwrap_or(LaidSource::Absent);
        let live = LaidSource::read(&lane.join(rel))?;
        v.inputs_read.insert(
            rel.clone(),
            over.get(rel).cloned().unwrap_or_else(|| live.clone()),
        );
        if SAME_FOR_ALL.contains(&rel.as_str()) {
            for f in [
                &mut v.released,
                &mut v.lane,
                &mut v.candidate,
                &mut v.clean,
                &mut v.attribution,
            ] {
                f.insert(rel.clone(), released.clone());
            }
            continue;
        }
        match over.get(rel) {
            Some(c) => {
                let eq = match (c, &released) {
                    (LaidSource::Bytes { bytes: a, .. }, LaidSource::Bytes { bytes: b, .. }) => {
                        a == b
                    }
                    (LaidSource::Absent, LaidSource::Absent) => true,
                    _ => false,
                };
                v.touched.insert(rel.clone(), eq);
                v.candidate.insert(rel.clone(), c.clone());
                v.clean.insert(rel.clone(), c.clone());
                v.attribution.insert(rel.clone(), released.clone());
            }
            None => {
                v.candidate.insert(rel.clone(), live.clone());
                v.clean.insert(rel.clone(), released.clone());
                v.attribution.insert(rel.clone(), live.clone());
            }
        }
        v.released.insert(rel.clone(), released);
        v.lane.insert(rel.clone(), live);
    }
    Ok(v)
}

fn subset(f: &Files, rels: &[String]) -> Files {
    f.iter()
        .filter(|(r, _)| rels.contains(r))
        .map(|(r, c)| (r.clone(), c.clone()))
        .collect()
}

/// Which variant an answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Var {
    Released,
    Clean,
    Lane,
    Candidate,
    Attribution,
}

/// One judged answer of one run (E-14 / E-15 judge two calls).
#[derive(Debug, Clone)]
struct Judged {
    /// suffix of the cell label (`""`, `"/2nd"`, `"/3rd"`).
    suffix: &'static str,
    answer: Answer,
    detail: String,
}

/// Shared state of one set of runs.
struct Runner<'a> {
    cfg: &'a ScreenConfig,
    root: PathBuf,
    until: Instant,
    counter: AtomicUsize,
    out_of_time: AtomicBool,
    leftovers: Mutex<Vec<u32>>,
    /// `<root>/py` (links to the interpreter) and the /usr/bin view without python (E-18).
    py_dir: Option<PathBuf>,
    nopy_usr_bin: PathBuf,
    /// Answers may come from (and go to) the process-wide cache (false on the BL1 repeat).
    use_cache: bool,
    /// RULINGS EL1 — the PLAN of this pass and how much of it ran to a result: every planned
    /// execution (`planned`), those that ran to an exit status within their time (`finished`), and
    /// early returns of a probe / I3 run before or after its executions (`broken`).
    planned: AtomicUsize,
    finished: AtomicUsize,
    broken: AtomicUsize,
    /// Test seam (EL1): the next execution of this pass fails to start.
    inject_spawn_failure: AtomicBool,
}

/// Process-wide answers of hook probes, by (device, the input bytes' group key, interpreter, class,
/// probe, set-up): RELEASED never changes inside one binary, and a lane's untouched devices answer the
/// same on every screen — only new bytes are run again. Memory only; bounded.
type CacheKey = (Device, String, String, &'static str, usize, Setup);
const CACHE_MAX: usize = 60_000;

fn answer_cache() -> &'static Mutex<HashMap<CacheKey, Vec<Judged>>> {
    static C: std::sync::OnceLock<Mutex<HashMap<CacheKey, Vec<Judged>>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_get(k: &CacheKey) -> Option<Vec<Judged>> {
    answer_cache().lock().ok().and_then(|m| m.get(k).cloned())
}

fn cache_put(k: CacheKey, v: Vec<Judged>) {
    if let Ok(mut m) = answer_cache().lock() {
        if m.len() >= CACHE_MAX {
            m.clear();
        }
        m.insert(k, v);
    }
}

impl<'a> Runner<'a> {
    /// RULINGS EL1: every planned execution of this pass ran to a result, nothing returned early,
    /// nothing ran out of time — computed from the plan, never from "time was left".
    fn complete(&self) -> bool {
        self.planned.load(Ordering::SeqCst) == self.finished.load(Ordering::SeqCst)
            && self.broken.load(Ordering::SeqCst) == 0
            && !self.out_of_time.load(Ordering::SeqCst)
    }

    fn mark_broken(&self) {
        self.broken.fetch_add(1, Ordering::SeqCst);
    }

    fn interp_key(&self) -> String {
        self.cfg
            .interpreter
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn new(cfg: &'a ScreenConfig, root: &Path, until: Instant) -> Result<Self, RepairError> {
        let py_dir = match &cfg.interpreter {
            Some(p) => Some(replay::interpreter_dir(root, p)?),
            None => None,
        };
        let nopy = root.join("nopy-usr-bin");
        make_nopy_usr_bin(&nopy)?;
        Ok(Self {
            cfg,
            root: root.to_path_buf(),
            until,
            counter: AtomicUsize::new(0),
            out_of_time: AtomicBool::new(false),
            leftovers: Mutex::new(Vec::new()),
            py_dir,
            nopy_usr_bin: nopy,
            use_cache: true,
            planned: AtomicUsize::new(0),
            finished: AtomicUsize::new(0),
            broken: AtomicUsize::new(0),
            inject_spawn_failure: AtomicBool::new(false),
        })
    }

    fn fresh_dir(&self) -> Result<PathBuf, RepairError> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let d = self.root.join(format!("r{n}"));
        std::fs::create_dir_all(&d)
            .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
        Ok(d)
    }

    fn child_deadline(&self) -> Option<Duration> {
        // out of time is sticky: no child starts after the pass ran out of its time
        if self.out_of_time.load(Ordering::SeqCst) {
            return None;
        }
        let left = replay::left(self.until);
        if left < replay::MIN_CHILD_SLICE {
            self.out_of_time.store(true, Ordering::SeqCst);
            return None;
        }
        Some(left.min(self.cfg.child_cap))
    }

    fn run(
        &self,
        argv: &[String],
        env: &[(String, String)],
        cwd: &Path,
        stdin: &[u8],
    ) -> replay::ChildOutcome {
        self.planned.fetch_add(1, Ordering::SeqCst);
        let Some(d) = self.child_deadline() else {
            return replay::ChildOutcome {
                timed_out: true,
                ..Default::default()
            };
        };
        if self.inject_spawn_failure.swap(false, Ordering::SeqCst) {
            return replay::ChildOutcome {
                spawn_error: Some("injected spawn failure (test)".into()),
                ..Default::default()
            };
        }
        let o = replay::run_child(argv, env, cwd, stdin, d);
        if !o.timed_out && o.spawn_error.is_none() && (o.code.is_some() || o.signal.is_some()) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }
        if !o.leftovers.is_empty() {
            if let Ok(mut l) = self.leftovers.lock() {
                l.extend(o.leftovers.iter().copied());
            }
        }
        o
    }
}

/// A view of /usr/bin without any python (C4 E-18: "a PATH holding no python3, python or py").
fn make_nopy_usr_bin(dst: &Path) -> Result<(), RepairError> {
    std::fs::create_dir_all(dst).map_err(|e| RepairError::Io(format!("{}: {e}", dst.display())))?;
    #[cfg(unix)]
    {
        if let Ok(rd) = std::fs::read_dir("/usr/bin") {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with("python") || n.starts_with("pydoc") || n == "py" || n == "cys" {
                    continue;
                }
                let _ = std::os::unix::fs::symlink(e.path(), dst.join(&n));
            }
        }
    }
    Ok(())
}

fn lay_files(pack: &Path, files: &Files) -> Result<(), RepairError> {
    std::fs::create_dir_all(pack)
        .map_err(|e| RepairError::Io(format!("{}: {e}", pack.display())))?;
    for (rel, c) in files {
        replay::lay_out(&pack.join(rel), c)?;
    }
    Ok(())
}

fn touch(p: &Path) -> Result<(), RepairError> {
    std::fs::write(p, b"").map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))
}

fn subst(s: &str, map: &[(&str, String)]) -> String {
    let mut out = s.to_string();
    for (k, v) in map {
        out = out.replace(k, v);
    }
    out
}

fn subst_value(v: &mut serde_json::Value, map: &[(&str, String)]) {
    match v {
        serde_json::Value::String(s) => *s = subst(s, map),
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| subst_value(x, map)),
        serde_json::Value::Object(o) => o.values_mut().for_each(|x| subst_value(x, map)),
        _ => {}
    }
}

/// The hook input of a probe (stdin of the device).
fn probe_input(p: &Probe, map: &[(&str, String)], work: &Path) -> Vec<u8> {
    if p.raw {
        return p.input.as_bytes().to_vec();
    }
    let mut ti: serde_json::Value =
        serde_json::from_str(p.input).unwrap_or_else(|_| serde_json::json!({}));
    subst_value(&mut ti, map);
    if p.pad > 0 {
        if let Some(o) = ti.as_object_mut() {
            o.insert(
                "content".into(),
                serde_json::Value::String("x".repeat(p.pad)),
            );
        }
    }
    serde_json::json!({
        "session_id": "screen-probe",
        "transcript_path": "",
        "cwd": work.to_string_lossy(),
        "hook_event_name": "PreToolUse",
        "tool_name": p.tool,
        "tool_input": ti,
    })
    .to_string()
    .into_bytes()
}

/// DENY = exit 2, or exit 0 with a deny payload; ALLOW = exit 0; ERROR = anything else (AY1, BD5).
fn answer_of(o: &replay::ChildOutcome) -> (Answer, String) {
    if o.timed_out {
        return (Answer::Error, "timeout".into());
    }
    // RULINGS CZ4: a run that left a process or a held pipe behind is not measurable
    if let Some(why) = o.left_something() {
        return (Answer::Error, why);
    }
    if let Some(e) = &o.spawn_error {
        return (Answer::Error, format!("spawn: {e}"));
    }
    match o.code {
        Some(2) => (Answer::Deny, "exit 2".into()),
        Some(0) => {
            // RULINGS EL1: an answer is judged only when it was observed WHOLE — an output that hit
            // the cap or whose read failed is never a DENY-by-JSON, and it is no ALLOW either
            if !o.stdout_whole() {
                (Answer::Error, format!("exit 0, output not observed whole ({:?})", o.stdout_state))
            } else if deny_payload(&o.stdout) {
                (Answer::Deny, "exit 0 + deny payload".into())
            } else {
                (Answer::Allow, "exit 0".into())
            }
        }
        Some(c) => (Answer::Error, format!("exit {c}")),
        None => (Answer::Error, format!("signal {:?}", o.signal)),
    }
}

/// RULINGS DF1 / EO1 — DENY-by-JSON is a BYTE GRAMMAR, not a trim: the output must be, byte for
/// byte, optional JSON white space (0x20 0x09 0x0A 0x0D) + ONE object of exactly the shape the
/// released devices print — the only top-level key `hookSpecificOutput`, an object whose keys are
/// exactly `hookEventName` = `PreToolUse`, `permissionDecision` = `deny` and
/// `permissionDecisionReason`, every value a string, no key twice — + optional JSON white space.
/// Anything else (another white space such as U+0085, U+00A0 or U+FEFF, a second object, another
/// key, another value type) is "not shown".
pub(crate) fn deny_payload(stdout: &[u8]) -> bool {
    let json_ws = |b: &u8| matches!(*b, b' ' | b'\t' | b'\n' | b'\r');
    let start = stdout.iter().position(|b| !json_ws(b)).unwrap_or(stdout.len());
    let end = stdout.iter().rposition(|b| !json_ws(b)).map_or(start, |i| i + 1);
    let body = &stdout[start..end];
    if body.first() != Some(&b'{') {
        return false;
    }
    // a key twice is a shape the devices never print (a parser would keep one of them)
    let text = String::from_utf8_lossy(body);
    for key in [
        "\"hookSpecificOutput\"",
        "\"hookEventName\"",
        "\"permissionDecision\"",
        "\"permissionDecisionReason\"",
    ] {
        if text.matches(key).count() > 1 {
            return false;
        }
    }
    let Ok(serde_json::Value::Object(top)) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    if top.len() != 1 {
        return false;
    }
    let Some(serde_json::Value::Object(h)) = top.get("hookSpecificOutput") else {
        return false;
    };
    let keys: BTreeSet<&str> = h.keys().map(String::as_str).collect();
    keys == BTreeSet::from(["hookEventName", "permissionDecision", "permissionDecisionReason"])
        && h.values().all(|v| v.is_string())
        && h.get("hookEventName").and_then(|v| v.as_str()) == Some("PreToolUse")
        && h.get("permissionDecision").and_then(|v| v.as_str()) == Some("deny")
}

/// The budget counter file the gate keeps for a session (`<state>/capgate/<sha256(sid)[:32]>.calls`,
/// one byte per call) — C-23 seeds it at the budget.
fn seed_budget(home: &Path) -> Result<(), RepairError> {
    use sha2::{Digest, Sha256};
    let key: String = Sha256::digest(b"screen-probe")
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let d = home.join(".cys").join("state").join("capgate");
    std::fs::create_dir_all(&d).map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
    std::fs::write(d.join(format!("{key}.calls")), vec![1u8; 2000])
        .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))
}

/// One hook-probe run (a single call, or a sequence for E-13 … E-15) of `class`/`probe` in a cell
/// set-up, with `files` = the device's test inputs of one variant.
fn run_hook_probe(
    r: &Runner<'_>,
    files: &Files,
    released_files: &Files,
    class: &Class,
    probe: &Probe,
    setup: Setup,
) -> Vec<Judged> {
    match run_hook_probe_inner(r, files, released_files, class, probe, setup) {
        Ok(j) => j,
        Err(e) => {
            r.mark_broken();
            vec![Judged {
            suffix: "",
            answer: Answer::Error,
            detail: format!("layout: {e:?}"),
        }]
        }
    }
}

fn run_hook_probe_inner(
    r: &Runner<'_>,
    files: &Files,
    released_files: &Files,
    class: &Class,
    probe: &Probe,
    setup: Setup,
) -> Result<Vec<Judged>, RepairError> {
    let run = r.fresh_dir()?;
    let res = (|| -> Result<Vec<Judged>, RepairError> {
        let home = run.join("h");
        let tmp = run.join("t");
        let work = run.join("w");
        let bin = run.join("bin");
        for d in [&home, &tmp, &work, &home.join(".claude")] {
            std::fs::create_dir_all(d)
                .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
        }
        std::fs::write(home.join(".claude").join("settings.json"), "{}\n")
            .map_err(|e| RepairError::Io(e.to_string()))?;
        replay::write_stub(&bin, class.role, false)?;
        // the variant's pack and, where the cell needs it, a second lane's pack (RELEASED inputs)
        let (variant, other) = match setup {
            Setup::PackAtHome => (home.join(".cys").join("pack"), None),
            Setup::OtherLane
            | Setup::FromOtherLane
            | Setup::ActiveOtherLane
            | Setup::ActiveFromOtherLane => (
                run.join("p").join("pack"),
                Some(home.join(".cys").join("pack-dept-x")),
            ),
            _ => (run.join("p").join("pack"), None),
        };
        lay_files(&variant, files)?;
        if let Some(o) = &other {
            lay_files(o, released_files)?;
        }
        // flags (in the pack of the hook that decides)
        match setup {
            Setup::Active => touch(&variant.join("AUTOPILOT_ACTIVE"))?,
            Setup::Paused => touch(&variant.join("AUTOPILOT_PAUSED"))?,
            Setup::ActivePaused => {
                touch(&variant.join("AUTOPILOT_ACTIVE"))?;
                touch(&variant.join("AUTOPILOT_PAUSED"))?;
            }
            Setup::ActiveOtherLane => {
                if let Some(o) = &other {
                    touch(&o.join("AUTOPILOT_ACTIVE"))?;
                }
            }
            Setup::ActiveFromOtherLane => touch(&variant.join("AUTOPILOT_ACTIVE"))?,
            _ => {}
        }
        if probe.seed_budget {
            seed_budget(&home)?;
        }
        // which pack the hook runs from, and which pack CYS_PACK_DIR names
        let (hook_pack, pack_env): (PathBuf, Option<PathBuf>) = match setup {
            Setup::PackAtHome => (variant.clone(), None),
            Setup::OtherLane | Setup::ActiveOtherLane => (variant.clone(), other.clone()),
            Setup::FromOtherLane | Setup::ActiveFromOtherLane => (
                other.clone().unwrap_or_else(|| variant.clone()),
                Some(variant.clone()),
            ),
            _ => (variant.clone(), Some(variant.clone())),
        };
        let pack_named = pack_env
            .clone()
            .unwrap_or_else(|| home.join(".cys").join("pack"));
        let map: Vec<(&str, String)> = vec![
            ("{pack}", pack_named.to_string_lossy().into_owned()),
            ("{w}", work.to_string_lossy().into_owned()),
            ("{home}", home.to_string_lossy().into_owned()),
            (
                "{other}",
                home.join(".cys")
                    .join("pack-dept-zz")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ];
        let sock = ScratchSocket::bind(&run)?;
        let nosock = run.join("nosock");
        // PATH: the stub, the interpreter's private folder, the system folders (E-3 no stub; E-18 no
        // interpreter and /usr/bin seen without python)
        let mut path: Vec<String> = Vec::new();
        if setup != Setup::NoCys {
            path.push(bin.to_string_lossy().into_owned());
        }
        if setup != Setup::NoInterpreter {
            if let Some(p) = &r.py_dir {
                path.push(p.to_string_lossy().into_owned());
            }
            path.push("/usr/bin".into());
        } else {
            path.push(r.nopy_usr_bin.to_string_lossy().into_owned());
        }
        path.extend(["/bin", "/usr/sbin", "/sbin"].map(String::from));
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let role_env = if setup == Setup::CacheEnvDisagree {
            "worker"
        } else {
            class.role
        };
        let mut env: Vec<(String, String)> = vec![
            ("HOME".into(), home.to_string_lossy().into_owned()),
            ("TMPDIR".into(), format!("{}/", tmp.to_string_lossy())),
            ("PATH".into(), path.join(":")),
            ("USER".into(), user.clone()),
            ("LOGNAME".into(), user),
            ("SHELL".into(), "/bin/sh".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("LC_ALL".into(), "en_US.UTF-8".into()),
            ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
            ("CYS_SURFACE_ID".into(), "7".into()),
            ("CYS_ROLE".into(), role_env.into()),
            (
                "CYS_SOCKET".into(),
                if setup == Setup::NoSocket {
                    nosock.to_string_lossy().into_owned()
                } else {
                    sock.path().to_string_lossy().into_owned()
                },
            ),
        ];
        if let Some(p) = &pack_env {
            env.push(("CYS_PACK_DIR".into(), p.to_string_lossy().into_owned()));
        }
        if setup != Setup::NoInterpreter {
            if let Some(p) = &r.cfg.interpreter {
                env.push(("CYS_PY".into(), p.to_string_lossy().into_owned()));
            }
        }
        if setup == Setup::TmpReadonly {
            replay::set_mode(&tmp, 0o555);
        }
        let argv = vec![
            "/bin/sh".to_string(),
            hook_pack
                .join(class.device.rel())
                .to_string_lossy()
                .into_owned(),
        ];
        let input = probe_input(probe, &map, &work);
        let call = |env: &[(String, String)]| r.run(&argv, env, &work, &input);
        let judged = match setup {
            Setup::Warm => {
                let _ = call(&env);
                let (a, d) = answer_of(&call(&env));
                vec![Judged {
                    suffix: "",
                    answer: a,
                    detail: d,
                }]
            }
            Setup::CacheFallback | Setup::CacheEnvDisagree => {
                // first call answered by the stub (the role is cached), then the socket goes away
                let first_env: Vec<(String, String)> = env
                    .iter()
                    .map(|(k, v)| {
                        if k == "CYS_ROLE" {
                            (k.clone(), class.role.to_string())
                        } else {
                            (k.clone(), v.clone())
                        }
                    })
                    .collect();
                let _ = call(&first_env);
                let _ = std::fs::remove_file(sock.path());
                let (a2, d2) = answer_of(&call(&env));
                let (a3, d3) = answer_of(&call(&env));
                vec![
                    Judged {
                        suffix: "/2nd",
                        answer: a2,
                        detail: d2,
                    },
                    Judged {
                        suffix: "/3rd",
                        answer: a3,
                        detail: d3,
                    },
                ]
            }
            _ => {
                let (a, d) = answer_of(&call(&env));
                vec![Judged {
                    suffix: "",
                    answer: a,
                    detail: d,
                }]
            }
        };
        drop(sock);
        // the stub's call log proves the hook reached the screen's own stub (never a real cys)
        let calls = replay::stub_calls(&bin);
        let mut judged = judged;
        if let Some(last) = judged.last_mut() {
            let shown: Vec<String> = calls
                .iter()
                .take(4)
                .map(|c| c.chars().take(200).collect())
                .collect();
            last.detail = format!(
                "{}; stub calls {}: {}",
                last.detail,
                calls.len(),
                shown.join(" | ")
            );
        }
        Ok(judged)
    })();
    // RULINGS CZ4: a cleanup error / a leftover scratch root makes the probe not measurable
    let cleaned = replay::remove_tree(&run);
    res.and_then(|j| cleaned.map(|()| j))
}

/// E-21 (BC3, BH1): the RELEASED guard.sh under its own no-Python switch with a variant's other
/// inputs. MEASURED only when the backstop's marker (G-22, G-23) and the deny reason appear on
/// standard error; otherwise NOT MEASURABLE (an ERROR answer).
fn run_backstop_probe(
    r: &Runner<'_>,
    files: &Files,
    released_guard: &LaidSource,
    probe: &Probe,
    active: bool,
    reason: &str,
) -> Judged {
    let go = || -> Result<Judged, RepairError> {
        let run = r.fresh_dir()?;
        let res = (|| -> Result<Judged, RepairError> {
            let home = run.join("h");
            let tmp = run.join("t");
            let work = run.join("w");
            let bin = run.join("bin");
            for d in [&home, &tmp, &work, &home.join(".claude")] {
                std::fs::create_dir_all(d)
                    .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
            }
            std::fs::write(home.join(".claude").join("settings.json"), "{}\n")
                .map_err(|e| RepairError::Io(e.to_string()))?;
            replay::write_stub(&bin, "worker", false)?;
            let pack = run.join("p").join("pack");
            let mut f = files.clone();
            f.insert(Device::Guard.rel().to_string(), released_guard.clone());
            lay_files(&pack, &f)?;
            if active {
                touch(&pack.join("AUTOPILOT_ACTIVE"))?;
            }
            let sock = ScratchSocket::bind(&run)?;
            let mut path: Vec<String> = vec![bin.to_string_lossy().into_owned()];
            if let Some(p) = &r.py_dir {
                path.push(p.to_string_lossy().into_owned());
            }
            path.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(String::from));
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            let mut env: Vec<(String, String)> = vec![
                ("HOME".into(), home.to_string_lossy().into_owned()),
                ("TMPDIR".into(), format!("{}/", tmp.to_string_lossy())),
                ("PATH".into(), path.join(":")),
                ("USER".into(), user.clone()),
                ("LOGNAME".into(), user),
                ("SHELL".into(), "/bin/sh".into()),
                ("LANG".into(), "en_US.UTF-8".into()),
                ("LC_ALL".into(), "en_US.UTF-8".into()),
                ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
                ("CYS_SURFACE_ID".into(), "7".into()),
                ("CYS_ROLE".into(), "worker".into()),
                (
                    "CYS_SOCKET".into(),
                    sock.path().to_string_lossy().into_owned(),
                ),
                ("CYS_PACK_DIR".into(), pack.to_string_lossy().into_owned()),
                // BA1's one exception (BC3): the RELEASED guard.sh's own switches
                ("GUARD_TEST_MODE".into(), "1".into()),
                ("GUARD_FORCE_NOPY".into(), "1".into()),
            ];
            if let Some(p) = &r.cfg.interpreter {
                env.push(("CYS_PY".into(), p.to_string_lossy().into_owned()));
            }
            let map: Vec<(&str, String)> = vec![
                ("{pack}", pack.to_string_lossy().into_owned()),
                ("{w}", work.to_string_lossy().into_owned()),
                ("{home}", home.to_string_lossy().into_owned()),
                (
                    "{other}",
                    home.join(".cys")
                        .join("pack-dept-zz")
                        .to_string_lossy()
                        .into_owned(),
                ),
            ];
            let argv = vec![
                "/bin/sh".to_string(),
                pack.join(Device::Guard.rel())
                    .to_string_lossy()
                    .into_owned(),
            ];
            let o = r.run(&argv, &env, &work, &probe_input(probe, &map, &work));
            drop(sock);
            let err = String::from_utf8_lossy(&o.stderr).into_owned();
            let marker_ok = active || err.lines().any(|l| l.trim_end() == BACKSTOP_MARKER);
            let reason_ok = err.lines().any(|l| l.trim_end() == reason);
            let (a, d) = answer_of(&o);
            Ok(if marker_ok && reason_ok && a == Answer::Deny {
                Judged {
                    suffix: "",
                    answer: Answer::Deny,
                    detail: "backstop ran (marker + reason)".into(),
                }
            } else if !marker_ok || !reason_ok {
                Judged {
                    suffix: "",
                    answer: Answer::Error,
                    detail: format!(
                        "backstop not proven ({d}; marker {marker_ok}, reason {reason_ok})"
                    ),
                }
            } else {
                Judged {
                    suffix: "",
                    answer: a,
                    detail: d,
                }
            })
        })();
        // RULINGS CZ4: a cleanup error / a leftover scratch root makes the probe not measurable
        let cleaned = replay::remove_tree(&run);
        res.and_then(|j| cleaned.map(|()| j))
    };
    go().unwrap_or_else(|e| {
        r.mark_broken();
        Judged {
            suffix: "",
            answer: Answer::Error,
            detail: format!("layout: {e:?}"),
        }
    })
}

/// Runs `jobs` on a small pool of threads.
fn run_pool<J: Send + Sync, R: Send>(
    threads: usize,
    jobs: &[J],
    f: impl Fn(&J) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(jobs.len()));
    std::thread::scope(|s| {
        for _ in 0..threads.max(1).min(jobs.len().max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(j) = jobs.get(i) else { break };
                let r = f(j);
                if let Ok(mut o) = out.lock() {
                    o.push((i, r));
                }
            });
        }
    });
    let mut v = out.into_inner().unwrap_or_else(|e| e.into_inner());
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

/// The answers of one variant for every (class, probe, setup) asked.
type AnswerKey = (&'static str, usize, Setup);

/// One device's hook-probe matrix across the variants (AY1 with BD1, BC5, BD5, BG1).
fn screen_device(
    r: &Runner<'_>,
    device: Device,
    v: &Variants,
    rels: &[String],
    results: &mut Vec<ProbeResult>,
) {
    let classes: Vec<&'static Class> = corpus::all_classes()
        .filter(|c| c.device == device && r.cfg.class_on(c.id))
        .collect();
    let cells: Vec<&'static Cell> = corpus::cells_of(device)
        .filter(|c| c.setup != Setup::Backstop && r.cfg.cell_on(c.id))
        .collect();
    let setups: BTreeSet<Setup> = cells.iter().map(|c| c.setup).collect();
    let files = |var: Var| -> Files {
        subset(
            match var {
                Var::Released => &v.released,
                Var::Clean => &v.clean,
                Var::Lane => &v.lane,
                Var::Candidate => &v.candidate,
                Var::Attribution => &v.attribution,
            },
            rels,
        )
    };
    let released_files = files(Var::Released);
    let released_key = group_key(&released_files);
    // answers per content group (variants with identical inputs share runs)
    let mut by_group: HashMap<String, HashMap<AnswerKey, Vec<Judged>>> = HashMap::new();
    let run_group =
        |var: Var,
         wanted: &BTreeSet<AnswerKey>,
         by_group: &mut HashMap<String, HashMap<AnswerKey, Vec<Judged>>>| {
            let f = files(var);
            let key = group_key(&f);
            let have = by_group.entry(key.clone()).or_default();
            // answers this process already measured for exactly these input bytes (never on the
            // after-run repeat — BL1 needs the runs to happen again)
            if r.use_cache {
                let missing: Vec<AnswerKey> = wanted
                    .iter()
                    .filter(|k| !have.contains_key(*k))
                    .copied()
                    .collect();
                for k in missing.iter() {
                    if let Some(j) =
                        cache_get(&(device, key.clone(), r.interp_key(), k.0, k.1, k.2))
                    {
                        have.insert(*k, j);
                    }
                }
            }
            let todo: Vec<AnswerKey> = wanted
                .iter()
                .filter(|k| !have.contains_key(*k))
                .copied()
                .collect();
            let got = run_pool(r.cfg.threads, &todo, |(cid, pi, setup)| {
                let class = classes.iter().find(|c| c.id == *cid).copied();
                match class.and_then(|c| c.probes.get(*pi).map(|p| (c, p))) {
                    Some((c, p)) => run_hook_probe(r, &f, &released_files, c, p, *setup),
                    None => Vec::new(),
                }
            });
            for (k, j) in todo.iter().zip(got.iter()) {
                // only an answer that was really observed is kept for later screens: never one of the
                // BL1 repeat (it runs WITHOUT the cache), never an ERROR (a time-out, a spawn failure,
                // a run that left something or an output not observed whole is no reusable fact)
                if r.use_cache && !j.iter().any(|x| x.answer == Answer::Error) {
                    cache_put(
                        (device, key.clone(), r.interp_key(), k.0, k.1, k.2),
                        j.clone(),
                    );
                }
            }
            let have = by_group.entry(key).or_default();
            for (k, j) in todo.into_iter().zip(got) {
                have.insert(k, j);
            }
        };
    // phase 1: RELEASED everywhere
    let mut all: BTreeSet<AnswerKey> = BTreeSet::new();
    for c in &classes {
        for pi in 0..c.probes.len() {
            for s in &setups {
                all.insert((c.id, pi, *s));
            }
        }
    }
    run_group(Var::Released, &all, &mut by_group);
    let rel_ans = by_group.get(&released_key).cloned().unwrap_or_default();
    // phase 2: CLEAN, LANE, CANDIDATE where RELEASED denies (a probe binds there)
    let binding: BTreeSet<AnswerKey> = all
        .iter()
        .filter(|k| {
            rel_ans
                .get(*k)
                .map_or(false, |j| j.iter().any(|x| x.answer == Answer::Deny))
        })
        .copied()
        .collect();
    for var in [Var::Clean, Var::Lane, Var::Candidate] {
        run_group(var, &binding, &mut by_group);
    }
    let get = |var: Var,
               k: &AnswerKey,
               by_group: &HashMap<String, HashMap<AnswerKey, Vec<Judged>>>|
     -> Option<Vec<Judged>> {
        by_group
            .get(&group_key(&files(var)))
            .and_then(|m| m.get(k))
            .cloned()
    };
    // phase 3: attribution where neither LANE nor CANDIDATE denies a binding probe
    let mut need_attr = BTreeSet::new();
    for k in &binding {
        let rj = rel_ans.get(k).cloned().unwrap_or_default();
        let lj = get(Var::Lane, k, &by_group).unwrap_or_default();
        let cj = get(Var::Candidate, k, &by_group).unwrap_or_default();
        let clj = get(Var::Clean, k, &by_group).unwrap_or_default();
        for (i, x) in rj.iter().enumerate() {
            if x.answer != Answer::Deny {
                continue;
            }
            let clean_denies = clj.get(i).map_or(false, |j| j.answer == Answer::Deny);
            let lane_denies = lj.get(i).map_or(false, |j| j.answer == Answer::Deny);
            let cand_denies = cj.get(i).map_or(false, |j| j.answer == Answer::Deny);
            if clean_denies && !lane_denies && !cand_denies {
                need_attr.insert(*k);
            }
        }
    }
    run_group(Var::Attribution, &need_attr, &mut by_group);
    // decisions
    let touched_inputs: Vec<(&String, bool)> = v
        .touched
        .iter()
        .filter(|(r, _)| rels.contains(r))
        .map(|(r, e)| (r, *e))
        .collect();
    let touches = !touched_inputs.is_empty();
    let all_released_equal = touches && touched_inputs.iter().all(|(_, e)| *e);
    for class in &classes {
        for (pi, _) in class.probes.iter().enumerate() {
            for cell in &cells {
                let k = (class.id, pi, cell.setup);
                let rj = rel_ans.get(&k).cloned().unwrap_or_default();
                for (i, rx) in rj.iter().enumerate() {
                    let pick =
                        |var: Var| get(var, &k, &by_group).and_then(|v| v.get(i).map(|j| j.answer));
                    let (clean, lane, cand, attr) = if rx.answer == Answer::Deny {
                        (
                            pick(Var::Clean),
                            pick(Var::Lane),
                            pick(Var::Candidate),
                            pick(Var::Attribution),
                        )
                    } else {
                        (None, None, None, None)
                    };
                    let decision = decide(
                        rx.answer,
                        clean,
                        lane,
                        cand,
                        attr,
                        touches,
                        all_released_equal,
                    );
                    results.push(ProbeResult {
                        test: class.test.to_string(),
                        class: if class.probes.len() > 1 {
                            format!("{}#{}", class.id, pi + 1)
                        } else {
                            class.id.to_string()
                        },
                        cell: format!("{}{}", cell.id, rx.suffix),
                        protection: protection_of(class.test).to_string(),
                        released: rx.answer,
                        clean,
                        lane,
                        candidate: cand,
                        attribution: attr,
                        decision,
                        detail: rx.detail.clone(),
                    });
                }
            }
        }
    }
}

/// AY1 (1)–(5) with BD1, BC5, BD5 and BG1 for one judged probe.
fn decide(
    released: Answer,
    clean: Option<Answer>,
    lane: Option<Answer>,
    cand: Option<Answer>,
    attr: Option<Answer>,
    touches: bool,
    all_released_equal: bool,
) -> ProbeDecision {
    let denies = |a: Option<Answer>| a == Some(Answer::Deny);
    let refuse = || {
        // BG1: a candidate state byte-equal to the RELEASED copy is never a ground for refusal
        if all_released_equal {
            ProbeDecision::LaneFinding
        } else {
            ProbeDecision::Refused
        }
    };
    match released {
        Answer::Allow => ProbeDecision::Unbound,
        Answer::Error => {
            if touches && !all_released_equal {
                ProbeDecision::Refused
            } else {
                ProbeDecision::NotMeasurable
            }
        }
        Answer::Deny => {
            if !denies(clean) {
                return refuse();
            }
            if denies(cand) {
                return ProbeDecision::Held;
            }
            if denies(lane) {
                return refuse();
            }
            if denies(attr) {
                return refuse();
            }
            ProbeDecision::LaneFinding
        }
    }
}

/// E-21 for a candidate that touches an input of guard.sh other than guard.sh itself (BC3).
fn screen_backstop(r: &Runner<'_>, v: &Variants, rels: &[String], results: &mut Vec<ProbeResult>) {
    let touches_other = v
        .touched
        .keys()
        .any(|t| rels.contains(t) && t != Device::Guard.rel());
    if !touches_other || !r.cfg.cell_on("E-21") {
        return;
    }
    let Some(released_guard) = v.released.get(Device::Guard.rel()).cloned() else {
        return;
    };
    let sub = |f: &Files| subset(f, rels);
    let all_released_equal = v
        .touched
        .iter()
        .filter(|(t, _)| rels.contains(t))
        .all(|(_, e)| *e);
    for (id, probe, active, reason) in BACKSTOP_CLASSES {
        if !r.cfg.class_on(id) {
            continue;
        }
        let jobs = [
            Var::Released,
            Var::Clean,
            Var::Lane,
            Var::Candidate,
            Var::Attribution,
        ];
        let got = run_pool(r.cfg.threads, &jobs, |var| {
            let f = sub(match var {
                Var::Released => &v.released,
                Var::Clean => &v.clean,
                Var::Lane => &v.lane,
                Var::Candidate => &v.candidate,
                Var::Attribution => &v.attribution,
            });
            run_backstop_probe(r, &f, &released_guard, probe, *active, reason)
        });
        let a = |i: usize| got.get(i).map(|j| j.answer);
        let released = a(0).unwrap_or(Answer::Error);
        // BH1: a variant run that does not prove the backstop is NOT MEASURABLE → refused (the cell
        // only runs for a candidate touching an input of guard.sh)
        let unproven = [1usize, 3].iter().any(|i| a(*i) == Some(Answer::Error));
        let mut decision = decide(released, a(1), a(2), a(3), a(4), true, all_released_equal);
        if released == Answer::Error || unproven {
            decision = if all_released_equal {
                ProbeDecision::NotMeasurable
            } else {
                ProbeDecision::Refused
            };
        }
        results.push(ProbeResult {
            test: "I4".into(),
            class: id.to_string(),
            cell: format!("E-21{}", if *active { "/active" } else { "" }),
            protection: protection_of("I4").to_string(),
            released,
            clean: a(1),
            lane: a(2),
            candidate: a(3),
            attribution: a(4),
            decision,
            detail: got
                .iter()
                .map(|j| j.detail.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        });
    }
}

/// Invariant I4's regions (AY4, BA4, BD5) — judged only for a candidate that touches guard.sh.
fn screen_i4(v: &Variants, results: &mut Vec<ProbeResult>) {
    let rel = Device::Guard.rel();
    let text_of = |c: Option<&LaidSource>| match c {
        Some(LaidSource::Bytes { bytes, .. }) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    };
    let released = text_of(v.released.get(rel));
    let touched = v.touched.contains_key(rel);
    let probe = if touched {
        v.candidate.get(rel)
    } else {
        v.lane.get(rel)
    };
    let other = text_of(probe);
    for (name, first, last) in corpus::I4_REGIONS {
        let r = released
            .as_deref()
            .and_then(|t| corpus::i4_region(t, first, last));
        let o = other
            .as_deref()
            .and_then(|t| corpus::i4_region(t, first, last));
        let (decision, detail) = match (&r, &o) {
            (None, _) => (
                ProbeDecision::NotMeasurable,
                "released anchor not found".to_string(),
            ),
            (Some(_), None) if touched => (
                ProbeDecision::Refused,
                "anchor not found in the candidate's guard.sh".into(),
            ),
            (Some(_), None) => (
                ProbeDecision::LaneFinding,
                "anchor not found in the lane's guard.sh".into(),
            ),
            (Some(a), Some(b)) if a == b => (
                ProbeDecision::Held,
                "byte-equal to the released copy".into(),
            ),
            (Some(_), Some(_)) if touched => (
                ProbeDecision::Refused,
                "region differs from the released copy".into(),
            ),
            (Some(_), Some(_)) => (
                ProbeDecision::LaneFinding,
                "the lane's region differs from the released copy".into(),
            ),
        };
        results.push(ProbeResult {
            test: "I4".into(),
            class: format!("I4:{name}"),
            cell: (*name).to_string(),
            protection: protection_of("I4").to_string(),
            released: if r.is_some() {
                Answer::Deny
            } else {
                Answer::Error
            },
            clean: None,
            lane: None,
            candidate: None,
            attribution: None,
            decision,
            detail,
        });
    }
}

/// AT4 on the FILE (BD3, BG1): a candidate table file holding a `capgate: deny` entry that the same
/// file's LANE state does not hold → refused, unless byte-equal to its RELEASED copy.
fn screen_at4(v: &Variants, results: &mut Vec<ProbeResult>) {
    for rel in ["state/hook-targets.json", "state/hook-targets.json.example"] {
        let Some(&released_equal) = v.touched.get(rel) else {
            continue;
        };
        let denies = |c: Option<&LaidSource>| -> BTreeSet<String> {
            let Some(LaidSource::Bytes { bytes, .. }) = c else {
                return BTreeSet::new();
            };
            let Ok(doc) = serde_json::from_slice::<serde_json::Value>(bytes) else {
                return BTreeSet::new();
            };
            doc.get("profiles")
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter(|e| {
                            e.pointer("/eligibility/capgate").and_then(|x| x.as_str())
                                == Some("deny")
                        })
                        .map(|e| {
                            e.get("basename")
                                .and_then(|b| b.as_str())
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let cand = denies(v.candidate.get(rel));
        let lane = denies(v.lane.get(rel));
        let added: Vec<&String> = cand.difference(&lane).collect();
        let decision = if added.is_empty() || released_equal {
            ProbeDecision::Held
        } else {
            ProbeDecision::Refused
        };
        results.push(ProbeResult {
            test: "I3".into(),
            class: format!("AT4:{rel}"),
            cell: "file".into(),
            protection: protection_of("I3").to_string(),
            released: Answer::Deny,
            clean: None,
            lane: None,
            candidate: None,
            attribution: None,
            decision,
            detail: if added.is_empty() {
                "no capgate deny added".into()
            } else {
                format!(
                    "capgate deny added for {added:?}{}",
                    if released_equal {
                        " (byte-equal to the released copy — BG1)"
                    } else {
                        ""
                    }
                )
            },
        });
    }
}

// ─────────────────────────────── invariant I3 — the registrar itself ───────────────────────────────

const CAPGATE_SCRIPT: &str = "role-capability-gate.sh";

/// The basenames named in a hook-target table (BA3).
fn table_basenames(c: Option<&LaidSource>) -> BTreeSet<String> {
    let Some(LaidSource::Bytes { bytes, .. }) = c else {
        return BTreeSet::new();
    };
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return BTreeSet::new();
    };
    doc.get("profiles")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("basename").and_then(|b| b.as_str()))
                .filter(|b| {
                    !b.is_empty()
                        && !b.contains('/')
                        && !b.contains('\\')
                        && *b != "."
                        && *b != ".."
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// BA3: `.claude` + every `.claude*` basename any variant's tables name.
fn profile_set(v: &Variants) -> Vec<String> {
    let mut s: BTreeSet<String> = BTreeSet::new();
    s.insert(".claude".into());
    for f in [&v.released, &v.lane, &v.candidate, &v.clean, &v.attribution] {
        for rel in ["state/hook-targets.json", "state/hook-targets.json.example"] {
            for b in table_basenames(f.get(rel)) {
                if b == ".claude" || b.starts_with(".claude-") {
                    s.insert(b);
                }
            }
        }
    }
    s.into_iter().collect()
}

/// The interpreters the gate (a `.sh` script) may be named after (RULINGS CZ1): the registrar
/// writes `sh <abs>` on Unix (`cysjavis-pack/bin/javis_preflight.py:_cys_hook_cmd`); a literal
/// absolute path of the same two programs is the same program. Nothing else — no flag, no wrapper.
const GATE_INTERPRETERS: &[&str] = &[
    "sh",
    "bash",
    "/bin/sh",
    "/bin/bash",
    "/usr/bin/sh",
    "/usr/bin/bash",
];

/// The least hook timeout (seconds) that lets the gate answer: the released registrar's declared
/// capgate timeout (`javis_preflight.py` `("role-capability-gate.sh", "PreToolUse"): 15`) — a
/// shorter one ends the hook before the gate decides, and an ended hook blocks nothing (CZ1).
pub(crate) const GATE_TIMEOUT_FLOOR_S: f64 = 15.0;

/// A character of a bare part of a hook command (the set of T3b's grammar,
/// `src/bin/cysd/alerts.rs:is_bare_char`, without `~`).
fn gate_bare_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-')
}

/// The words of a hook command when the WHOLE text is made of the closed grammar's words (RULINGS
/// CZ1 — the form of T3b's detector, `src/bin/cysd/alerts.rs:grammar_tokens`): bare characters, or
/// single / double quoted parts without `$`, a backquote, a backslash, `!` or a control character.
/// `None` at the first character or form outside it: a comment (`#`), an operator (`;` `&` `|`), a
/// redirection, a subshell, a glob, an expansion, a newline. Each word: (text, every part bare).
pub(crate) fn hook_command_words(src: &str) -> Option<Vec<(String, bool)>> {
    let cs: Vec<char> = src.chars().collect();
    let n = cs.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        if cs[i] == ' ' || cs[i] == '\t' {
            i += 1;
            continue;
        }
        let start = i;
        let (mut text, mut bare) = (String::new(), true);
        while i < n && cs[i] != ' ' && cs[i] != '\t' {
            match cs[i] {
                '\'' => {
                    bare = false;
                    i += 1;
                    while i < n && cs[i] != '\'' {
                        if cs[i].is_control() {
                            return None;
                        }
                        text.push(cs[i]);
                        i += 1;
                    }
                    if i >= n {
                        return None;
                    }
                    i += 1;
                }
                '"' => {
                    bare = false;
                    i += 1;
                    while i < n && cs[i] != '"' {
                        if matches!(cs[i], '$' | '`' | '\\' | '!') || cs[i].is_control() {
                            return None;
                        }
                        text.push(cs[i]);
                        i += 1;
                    }
                    if i >= n {
                        return None;
                    }
                    i += 1;
                }
                c if gate_bare_char(c) && !(c == '=' && i == start) => {
                    text.push(c);
                    i += 1;
                }
                _ => return None,
            }
        }
        out.push((text, bare));
    }
    Some(out)
}

fn executable_file(m: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        false
    }
}

/// Does this hook object EXECUTE the gate at `gate` (RULINGS CZ1 — "registered" means the protection
/// runs)? An executable command hook: `type` exactly `command`, no key the registrar does not write
/// (`type`, `command`, `timeout`), a timeout (when given) a number of at least
/// [`GATE_TIMEOUT_FLOOR_S`]; its command ONE simple command of the closed grammar — the gate by its
/// literal path, alone (then the file must be executable) or after one interpreter of
/// [`GATE_INTERPRETERS`] (bare) — and the gate a regular file. Every other form (a comment, a no-op
/// wrapper, a conditional, an argument, a redirection, another hook type) does not run it.
pub(crate) fn hook_runs_gate(h: &serde_json::Value, gate: &Path) -> bool {
    let Some(o) = h.as_object() else {
        return false;
    };
    if o
        .keys()
        .any(|k| !matches!(k.as_str(), "type" | "command" | "timeout"))
    {
        return false;
    }
    if o.get("type").and_then(|t| t.as_str()) != Some("command") {
        return false;
    }
    if let Some(t) = o.get("timeout") {
        if !t.as_f64().is_some_and(|s| s >= GATE_TIMEOUT_FLOOR_S) {
            return false;
        }
    }
    let Some(words) = o
        .get("command")
        .and_then(|c| c.as_str())
        .and_then(hook_command_words)
    else {
        return false;
    };
    let want = gate.to_string_lossy();
    let meta = std::fs::metadata(gate).ok();
    let is_file = meta.as_ref().is_some_and(|m| m.is_file());
    match words.as_slice() {
        [(p, _)] => *p == *want && meta.as_ref().is_some_and(executable_file),
        [(interp, true), (p, _)] => {
            GATE_INTERPRETERS.contains(&interp.as_str()) && *p == *want && is_file
        }
        _ => false,
    }
}

/// Is the capability gate registered in this settings file — does a `PreToolUse` entry for every
/// tool (no matcher, or the empty one — C28's scope rule) hold a hook that EXECUTES it (RULINGS CZ1,
/// [`hook_runs_gate`])? `disableAllHooks: true` runs no hook at all. `None` = the file is not
/// readable JSON.
fn gate_registered(settings: &Path, pack: &Path) -> Option<bool> {
    let text = replay::read_bounded(settings).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&text).ok()?;
    if v.get("disableAllHooks").and_then(|d| d.as_bool()) == Some(true) {
        return Some(false);
    }
    let gate = pack.join("hooks").join(CAPGATE_SCRIPT);
    let Some(entries) = v.pointer("/hooks/PreToolUse").and_then(|e| e.as_array()) else {
        return Some(false);
    };
    Some(entries.iter().any(|e| {
        let Some(eo) = e.as_object() else {
            return false;
        };
        eo.keys().all(|k| matches!(k.as_str(), "matcher" | "hooks"))
            && eo.get("matcher").map_or(true, |m| m.as_str() == Some(""))
            && eo
                .get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|hs| hs.iter().any(|h| hook_runs_gate(h, &gate)))
    }))
}

/// RULINGS EL2 — the execution signature of the gate's registration in a settings document: for every
/// `PreToolUse` entry holding a hook whose command names `gate`, the entry's own keys other than
/// `hooks` (the matcher …) and that hook object WHOLE (type, command, timeout, async, any other key).
fn gate_entries(v: &serde_json::Value, gate: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for e in v
        .pointer("/hooks/PreToolUse")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
    {
        let Some(eo) = e.as_object() else {
            continue;
        };
        let mut base = eo.clone();
        base.remove("hooks");
        for h in eo.get("hooks").and_then(|h| h.as_array()).into_iter().flatten() {
            if h.get("command")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains(gate))
            {
                let mut sig = base.clone();
                sig.insert("hook".into(), h.clone());
                out.push(serde_json::Value::Object(sig));
            }
        }
    }
    out
}

/// RULINGS EL2 — is the gate's registration in `settings` THE ENTRY THE RELEASED REGISTRAR WRITES
/// (`expected`, from the released template), compared as a whole — every execution attribute equal
/// and no other registration of the gate beside it? `None` = unreadable.
fn gate_entry_as_released(settings: &Path, gate: &str, expected: &serde_json::Value) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_slice(&replay::read_bounded(settings).ok()?).ok()?;
    if v.get("disableAllHooks").and_then(|d| d.as_bool()) == Some(true) {
        return Some(false);
    }
    let got = gate_entries(&v, gate);
    Some(!got.is_empty() && got.iter().all(|g| g == expected))
}

/// RULINGS DF2: the binding probe a registered command is executed with — a gate class the released
/// gate denies in the baseline set-up (a reviewer editing a repository file) and the released guard
/// does not, so a DENY is the gate's.
const I3_BINDING_CLASS: &str = "R-4";

/// What executing a registered command needs: the run's scratch pieces (replay rules).
struct ExecCtx<'p> {
    home: &'p Path,
    tmp: &'p Path,
    work: &'p Path,
    bin: &'p Path,
    sock: &'p Path,
    pack: &'p Path,
}

/// RULINGS DF2 — "registered" is judged by BEHAVIOUR: every `PreToolUse` command hook for all tools in
/// this settings file whose command names the gate's path (the pre-check; CZ1's closed grammar is
/// only reported) is EXECUTED the way the harness runs a hook — `/bin/sh -c <command>`, the binding
/// probe on standard input — under the replay rules (the run's scratch HOME, pack, stub `cys` and
/// socket, the fixed environment). Registered = one of them DENIES in DF1's reading. `None` = not
/// measurable (the settings unreadable, no interpreter, a run that could not start, ran out of time
/// or left something behind, and no deny elsewhere). The stub answers the probe's role during the
/// probe and the registrar's again afterwards.
fn gate_executes(r: &Runner<'_>, settings: &Path, x: &ExecCtx<'_>, registrar_stub: (&str, bool)) -> Option<bool> {
    let text = replay::read_bounded(settings).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&text).ok()?;
    if v.get("disableAllHooks").and_then(|d| d.as_bool()) == Some(true) {
        return Some(false);
    }
    let gate = x.pack.join("hooks").join(CAPGATE_SCRIPT);
    let gate_s = gate.to_string_lossy().into_owned();
    let commands: Vec<String> = v
        .pointer("/hooks/PreToolUse")
        .and_then(|e| e.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e.get("matcher").map_or(true, |m| m.as_str() == Some("")))
                .filter_map(|e| e.get("hooks").and_then(|h| h.as_array()))
                .flatten()
                .filter(|h| h.get("type").and_then(|t| t.as_str()) == Some("command"))
                .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
                .filter(|c| c.contains(&gate_s))
                .take(4)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if commands.is_empty() {
        return Some(false);
    }
    let py = r.cfg.interpreter.as_ref()?;
    let class = corpus::CORPUS_R.iter().find(|c| c.id == I3_BINDING_CLASS)?;
    let probe = class.probes.first()?;
    replay::write_stub_conf(x.bin, class.role, false).ok()?;
    let map: Vec<(&str, String)> = vec![
        ("{pack}", x.pack.to_string_lossy().into_owned()),
        ("{w}", x.work.to_string_lossy().into_owned()),
        ("{home}", x.home.to_string_lossy().into_owned()),
        (
            "{other}",
            x.home
                .join(".cys")
                .join("pack-dept-zz")
                .to_string_lossy()
                .into_owned(),
        ),
    ];
    let input = probe_input(probe, &map, x.work);
    let mut path_dirs = vec![x.bin.to_path_buf()];
    if let Some(p) = &r.py_dir {
        path_dirs.push(p.clone());
    }
    let mut env = replay::base_env(x.home, x.tmp, &path_dirs);
    env.push(("CYS_SURFACE_ID".into(), "7".into()));
    env.push(("CYS_ROLE".into(), class.role.into()));
    env.push(("CYS_SOCKET".into(), x.sock.to_string_lossy().into_owned()));
    env.push(("CYS_PACK_DIR".into(), x.pack.to_string_lossy().into_owned()));
    env.push(("CYS_PY".into(), py.to_string_lossy().into_owned()));
    let mut measurable = true;
    let mut denied = false;
    for c in &commands {
        let o = r.run(&["/bin/sh".to_string(), "-c".to_string(), c.clone()], &env, x.work, &input);
        if o.timed_out || o.spawn_error.is_some() || o.left_something().is_some() {
            measurable = false;
            continue;
        }
        if o.code == Some(2) || (o.code == Some(0) && deny_payload(&o.stdout)) {
            denied = true;
            break;
        }
    }
    replay::write_stub_conf(x.bin, registrar_stub.0, registrar_stub.1).ok()?;
    if denied {
        Some(true)
    } else if measurable {
        Some(false)
    } else {
        None
    }
}

/// The settings text with every hook command that runs the gate removed ("starts unregistered").
fn without_gate(template: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(template) else {
        return template.to_string();
    };
    if let Some(hooks) = v.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (_, entries) in hooks.iter_mut() {
            if let Some(arr) = entries.as_array_mut() {
                for e in arr.iter_mut() {
                    if let Some(hs) = e.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                        hs.retain(|h| {
                            !h.get("command")
                                .and_then(|c| c.as_str())
                                .map_or(false, |c| c.contains(CAPGATE_SCRIPT))
                        });
                    }
                }
                arr.retain(|e| {
                    e.get("hooks")
                        .and_then(|h| h.as_array())
                        .map_or(true, |hs| !hs.is_empty())
                });
            }
        }
    }
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| template.to_string())
}

/// The other product hooks of the released product (boot-critical hook rows and their events), as
/// a settings document — the shape the RELEASED registrar adds the gate to.
fn other_product_hooks(pack_placeholder: &str) -> String {
    let mut hooks: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for e in super::boot_critical::entries() {
        if e.category != super::boot_critical::EntryCategory::Hook
            || e.rel.ends_with(CAPGATE_SCRIPT)
        {
            continue;
        }
        for ev in e
            .check_arg
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            hooks.entry(ev.to_string()).or_default().push(serde_json::json!({
                "hooks": [{"type": "command", "command": format!("sh {pack_placeholder}/{}", e.rel)}]
            }));
        }
    }
    serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
        .unwrap_or_else(|_| "{}".into())
}

const PACK_PLACEHOLDER: &str = "@@SCREEN_PACK@@";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Layout {
    L1,
    L2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Scenario {
    S1,
    S2,
    S3,
}

/// The I3 run of one variant in one (layout, scenario): two registrar runs, the second on the
/// first's end state. `Some(ended registered)` per (run, target label); `None` = the run errored.
fn run_i3(
    r: &Runner<'_>,
    files: &Files,
    profiles: &[String],
    template: &str,
    layout: Layout,
    scenario: Scenario,
) -> Vec<(usize, String, Option<bool>, String)> {
    let mut out = Vec::new();
    let go = (|| -> Result<(), RepairError> {
        let run = r.fresh_dir()?;
        let res = (|| -> Result<(), RepairError> {
            let home = run.join("h");
            let tmp = run.join("t");
            let work = run.join("w");
            let bin = run.join("bin");
            for d in [&home, &tmp, &work] {
                std::fs::create_dir_all(d)
                    .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
            }
            let (pack, acct) = match layout {
                Layout::L1 => (run.join("x").join("pack"), None),
                Layout::L2 => (
                    run.join("x").join("pack-dept-x"),
                    Some(run.join("x").join("acct-dept-x")),
                ),
            };
            lay_files(&pack, files)?;
            let start = match scenario {
                Scenario::S3 => without_gate(template),
                _ => template.to_string(),
            }
            .replace(PACK_PLACEHOLDER, &pack.to_string_lossy());
            // RULINGS EL2: the entry the RELEASED registrar writes for the gate, in this layout
            let gate_path = pack.join("hooks").join(CAPGATE_SCRIPT).to_string_lossy().into_owned();
            let expected: Option<serde_json::Value> = serde_json::from_str::<serde_json::Value>(
                &template.replace(PACK_PLACEHOLDER, &pack.to_string_lossy()),
            )
            .ok()
            .and_then(|v| gate_entries(&v, &gate_path).into_iter().next());
            let mut targets: Vec<(String, PathBuf)> = Vec::new();
            for p in profiles {
                let d = home.join(p);
                std::fs::create_dir_all(&d)
                    .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
                std::fs::write(d.join("settings.json"), &start)
                    .map_err(|e| RepairError::Io(e.to_string()))?;
                if layout == Layout::L1 {
                    targets.push((p.clone(), d.join("settings.json")));
                }
            }
            match &acct {
                None => {
                    let d = run.join("x").join("claude");
                    std::fs::create_dir_all(&d)
                        .map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
                    std::fs::write(d.join("settings.json"), &start)
                        .map_err(|e| RepairError::Io(e.to_string()))?;
                    targets.push(("claude".into(), d.join("settings.json")));
                }
                Some(a) => {
                    std::fs::create_dir_all(a)
                        .map_err(|e| RepairError::Io(format!("{}: {e}", a.display())))?;
                    std::fs::write(a.join("settings.json"), &start)
                        .map_err(|e| RepairError::Io(e.to_string()))?;
                    targets.push(("acct-dept-x".into(), a.join("settings.json")));
                }
            }
            replay::write_stub(&bin, "worker", scenario != Scenario::S2)?;
            let sock = ScratchSocket::bind(&run)?;
            let mut path_dirs = vec![bin.clone()];
            if let Some(p) = &r.py_dir {
                path_dirs.push(p.clone());
            }
            let mut env = replay::base_env(&home, &tmp, &path_dirs);
            env.push(("CYS_PACK_DIR".into(), pack.to_string_lossy().into_owned()));
            env.push((
                "CYS_SOCKET".into(),
                sock.path().to_string_lossy().into_owned(),
            ));
            env.push(("PYTHONDONTWRITEBYTECODE".into(), "1".into()));
            if let Some(a) = &acct {
                env.push(("CYS_ACCOUNT_DIR".into(), a.to_string_lossy().into_owned()));
            }
            let Some(py) = &r.cfg.interpreter else {
                for k in 1..=2 {
                    for (label, _) in &targets {
                        out.push((k, label.clone(), None, "no interpreter".into()));
                    }
                }
                return Ok(());
            };
            env.push(("CYS_PY".into(), py.to_string_lossy().into_owned()));
            let argv = vec![
                py.to_string_lossy().into_owned(),
                pack.join("bin/javis_preflight.py")
                    .to_string_lossy()
                    .into_owned(),
                "--fix".into(),
                "--only".into(),
                "C28.self-correction".into(),
                "--json".into(),
            ];
            for k in 1..=2 {
                let o = r.run(&argv, &env, &work, b"");
                let completed = !o.timed_out
                    && o.spawn_error.is_none()
                    && o.left_something().is_none()
                    && matches!(o.code, Some(0) | Some(1));
                let why = if completed {
                    format!("exit {:?}", o.code)
                } else {
                    format!(
                        "registrar did not complete (exit {:?}, signal {:?}, timeout {}, left {:?}): {}",
                        o.code,
                        o.signal,
                        o.timed_out,
                        o.left_something(),
                        String::from_utf8_lossy(&o.stderr)
                            .chars()
                            .rev()
                            .take(300)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                    )
                };
                let x = ExecCtx {
                    home: &home,
                    tmp: &tmp,
                    work: &work,
                    bin: &bin,
                    sock: sock.path(),
                    pack: &pack,
                };
                for (label, t) in &targets {
                    // RULINGS EL2 / CZ1 / DF2: registered = the entry the released registrar writes
                    // (compared whole) AND the closed grammar AND the command really denies
                    let (st, detail) = if completed {
                        let whole = match &expected {
                            Some(e) => gate_entry_as_released(t, &gate_path, e),
                            None => None,
                        };
                        let grammar = gate_registered(t, &pack);
                        let st = match (whole, grammar) {
                            (Some(true), Some(true)) => {
                                gate_executes(r, t, &x, ("worker", scenario != Scenario::S2))
                            }
                            (Some(_), Some(_)) => Some(false),
                            _ => None,
                        };
                        (
                            st,
                            format!("{why}; entry as released {whole:?}, grammar {grammar:?}"),
                        )
                    } else {
                        (None, why.clone())
                    };
                    out.push((k, label.clone(), st, detail));
                }
            }
            drop(sock);
            Ok(())
        })();
        // RULINGS CZ4: a cleanup error / a leftover scratch root makes every row of the run not
        // measurable
        let cleaned = replay::remove_tree(&run);
        res.and(cleaned)
    })();
    if let Err(e) = go {
        r.mark_broken();
        for row in out.iter_mut() {
            row.2 = None;
            row.3 = format!("{} — run not clean: {e:?}", row.3);
        }
        out.push((1, "*".into(), None, format!("layout / cleanup: {e:?}")));
    }
    out
}

/// The "registered" starting shape (BH5): the RELEASED registrar's own registration in an s1 set-up
/// over the other product hooks, with the pack path replaced by a placeholder. `None` = the RELEASED
/// registrar did not register the gate there (every I3 probe is then NOT MEASURABLE).
fn i3_template(r: &Runner<'_>, released: &Files) -> Option<String> {
    let py = r.cfg.interpreter.as_ref()?;
    let run = r.fresh_dir().ok()?;
    let res = (|| -> Option<String> {
        let home = run.join("h");
        let tmp = run.join("t");
        let work = run.join("w");
        let bin = run.join("bin");
        for d in [&home, &tmp, &work, &home.join(".claude")] {
            std::fs::create_dir_all(d).ok()?;
        }
        let pack = run.join("x").join("pack");
        lay_files(&pack, released).ok()?;
        std::fs::create_dir_all(run.join("x").join("claude")).ok()?;
        let start = other_product_hooks(&pack.to_string_lossy());
        std::fs::write(home.join(".claude").join("settings.json"), &start).ok()?;
        replay::write_stub(&bin, "worker", true).ok()?;
        let sock = ScratchSocket::bind(&run).ok()?;
        let mut path_dirs = vec![bin.clone()];
        if let Some(p) = &r.py_dir {
            path_dirs.push(p.clone());
        }
        let mut env = replay::base_env(&home, &tmp, &path_dirs);
        env.push(("CYS_PACK_DIR".into(), pack.to_string_lossy().into_owned()));
        env.push((
            "CYS_SOCKET".into(),
            sock.path().to_string_lossy().into_owned(),
        ));
        env.push(("CYS_PY".into(), py.to_string_lossy().into_owned()));
        env.push(("PYTHONDONTWRITEBYTECODE".into(), "1".into()));
        let argv = vec![
            py.to_string_lossy().into_owned(),
            pack.join("bin/javis_preflight.py")
                .to_string_lossy()
                .into_owned(),
            "--fix".into(),
            "--only".into(),
            "C28.self-correction".into(),
            "--json".into(),
        ];
        let o = r.run(&argv, &env, &work, b"");
        if !matches!(o.code, Some(0) | Some(1)) || o.left_something().is_some() {
            return None;
        }
        let settings = home.join(".claude").join("settings.json");
        let x = ExecCtx {
            home: &home,
            tmp: &tmp,
            work: &work,
            bin: &bin,
            sock: sock.path(),
            pack: &pack,
        };
        // RULINGS DF2: the released registrar's registration must really deny the binding probe
        if gate_executes(r, &settings, &x, ("worker", true)) != Some(true) {
            return None;
        }
        drop(sock);
        let text = String::from_utf8(replay::read_bounded(&settings).ok()?).ok()?;
        Some(text.replace(&*pack.to_string_lossy(), PACK_PLACEHOLDER))
    })();
    // RULINGS CZ4: an unclean run gives no template (every I3 probe is then not measurable)
    replay::remove_tree(&run).ok()?;
    res
}

type I3Rows = Vec<(usize, String, Option<bool>, String)>;

fn i3_cache() -> &'static Mutex<HashMap<String, I3Rows>> {
    static C: std::sync::OnceLock<Mutex<HashMap<String, I3Rows>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn screen_i3(r: &Runner<'_>, v: &Variants, rels: &[String], results: &mut Vec<ProbeResult>) {
    // RULINGS DF2: the registered command is EXECUTED — the gate needs its prelude beside it
    let mut laid: Vec<String> = rels.to_vec();
    if !laid.iter().any(|x| x == "hooks/_lib.sh") {
        laid.push("hooks/_lib.sh".into());
    }
    let sub = |f: &Files| subset(f, &laid);
    let released_files = sub(&v.released);
    let touched: Vec<(&String, bool)> = v
        .touched
        .iter()
        .filter(|(t, _)| rels.contains(t))
        .map(|(t, e)| (t, *e))
        .collect();
    let touches = !touched.is_empty();
    let all_released_equal = touches && touched.iter().all(|(_, e)| *e);
    let template = i3_template(r, &released_files);
    let profiles = profile_set(v);
    let combos: Vec<(Layout, Scenario)> = [Layout::L1, Layout::L2]
        .iter()
        .flat_map(|l| [Scenario::S1, Scenario::S2, Scenario::S3].map(move |s| (*l, s)))
        .collect();
    let Some(template) = template else {
        r.mark_broken();
        for (l, s) in &combos {
            results.push(ProbeResult {
                test: "I3".into(),
                class: "I3:*".into(),
                cell: format!("{l:?}/{s:?}"),
                protection: protection_of("I3").to_string(),
                released: Answer::Error,
                clean: None,
                lane: None,
                candidate: None,
                attribution: None,
                decision: if touches && !all_released_equal {
                    ProbeDecision::Refused
                } else {
                    ProbeDecision::NotMeasurable
                },
                detail: "the released registrar did not produce the registered shape".into(),
            });
        }
        return;
    };
    // per distinct content group: every (layout, scenario)
    let groups: Vec<(Var, Files)> = [
        Var::Released,
        Var::Clean,
        Var::Lane,
        Var::Candidate,
        Var::Attribution,
    ]
    .iter()
    .map(|var| {
        (
            *var,
            sub(match var {
                Var::Released => &v.released,
                Var::Clean => &v.clean,
                Var::Lane => &v.lane,
                Var::Candidate => &v.candidate,
                Var::Attribution => &v.attribution,
            }),
        )
    })
    .collect();
    let mut keys: Vec<(String, Files)> = Vec::new();
    for (_, f) in &groups {
        let k = group_key(f);
        if !keys.iter().any(|(x, _)| *x == k) {
            keys.push((k, f.clone()));
        }
    }
    let jobs: Vec<(usize, Layout, Scenario)> = (0..keys.len())
        .flat_map(|g| combos.iter().map(move |(l, s)| (g, *l, *s)))
        .collect();
    // the registrar's end states for the same bytes, profile set and starting shape are the same on
    // every screen of this process (never reused on the BL1 repeat)
    let ctx_key = format!(
        "{}|{}|{}",
        profiles.join(","),
        group_key(&BTreeMap::from([(
            "template".to_string(),
            LaidSource::Bytes {
                bytes: template.as_bytes().to_vec(),
                mode: 0
            }
        )])),
        r.interp_key()
    );
    let got = run_pool(r.cfg.threads, &jobs, |(g, l, s)| {
        let ck = format!("{}|{l:?}|{s:?}|{ctx_key}", keys[*g].0);
        if r.use_cache {
            if let Some(rows) = i3_cache().lock().ok().and_then(|m| m.get(&ck).cloned()) {
                return rows;
            }
        }
        let rows = run_i3(r, &keys[*g].1, &profiles, &template, *l, *s);
        // never kept from the BL1 repeat (which runs without the cache), never a not-measured row
        if r.use_cache && rows.iter().all(|(_, _, st, _)| st.is_some()) {
            if let Ok(mut m) = i3_cache().lock() {
                if m.len() > 2_000 {
                    m.clear();
                }
                m.insert(ck, rows.clone());
            }
        }
        rows
    });
    let mut table: HashMap<(String, Layout, Scenario), Vec<(usize, String, Option<bool>, String)>> =
        HashMap::new();
    for ((g, l, s), rows) in jobs.iter().zip(got) {
        table.insert((keys[*g].0.clone(), *l, *s), rows);
    }
    let key_of = |var: Var| {
        group_key(
            &groups
                .iter()
                .find(|(x, _)| *x == var)
                .map(|(_, f)| f.clone())
                .unwrap_or_default(),
        )
    };
    let ans = |b: Option<bool>| match b {
        Some(true) => Answer::Deny, // "registered" plays the part of a deny: the protection holds
        Some(false) => Answer::Allow,
        None => Answer::Error,
    };
    for (l, s) in &combos {
        let rows_of = |var: Var| {
            table
                .get(&(key_of(var), *l, *s))
                .cloned()
                .unwrap_or_default()
        };
        let rel_rows = rows_of(Var::Released);
        for (k, label, st, why) in &rel_rows {
            let pick = |var: Var| -> Option<Answer> {
                rows_of(var)
                    .iter()
                    .find(|(k2, l2, _, _)| k2 == k && l2 == label)
                    .map(|(_, _, b, _)| ans(*b))
            };
            let released = ans(*st);
            let (clean, lane, cand, attr) = if released == Answer::Deny {
                (
                    pick(Var::Clean),
                    pick(Var::Lane),
                    pick(Var::Candidate),
                    pick(Var::Attribution),
                )
            } else {
                (None, None, None, None)
            };
            let decision = decide(
                released,
                clean,
                lane,
                cand,
                attr,
                touches,
                all_released_equal,
            );
            results.push(ProbeResult {
                test: "I3".into(),
                class: format!("I3:{label}"),
                cell: format!("{l:?}/{s:?}/run{k}"),
                protection: protection_of("I3").to_string(),
                released,
                clean,
                lane,
                candidate: cand,
                attribution: attr,
                decision,
                detail: why.clone(),
            });
        }
    }
}

// ─────────────────────────────────────────── the screen ───────────────────────────────────────────

/// The screen with an explicit configuration (production: [`screen_protections`]).
pub(crate) fn screen_with(
    cfg: &ScreenConfig,
    cand: &ScreenCandidate,
    lane: &Path,
    deadline: Duration,
) -> Result<ScreenReport, RepairError> {
    let started = Instant::now();
    let until = started + deadline;
    replay::system_path_has_no_cys()?;
    if !lane.is_absolute() {
        return Err(RepairError::Precondition(format!(
            "{}: 레인의 팩 위치가 절대 경로가 아닙니다",
            lane.display()
        )));
    }
    let inputs = inputs_by_test()?;
    let rels = all_input_rels()?;
    let v = build_variants(cfg, cand, lane, &rels)?;
    // RULINGS CZ5: the fingerprint of the inputs the probes run on — never a second live read
    let fingerprint = fingerprint_of_read(&v);
    if let Some(hook) = &cfg.after_inputs_read {
        hook();
    }
    // the scoped after-run comparison (BK1): before
    let settings = replay::real_settings_targets(lane, &cfg.real_home);
    let before = replay::live_snapshot(lane, &settings);
    if let Some(hook) = &cfg.between_snapshots {
        hook(1);
    }
    let (mut results, leftovers, out_of_time, _first_complete) =
        run_all(cfg, &v, &inputs, until, true)?;
    let after = replay::live_snapshot(lane, &settings);
    let first: Vec<LiveDifference> = replay::changed_items(&before, &after)
        .into_iter()
        .map(|(b, a)| LiveDifference {
            item: a.item,
            before: format!("{} [{}]", b.state, b.stamp),
            after: format!("{} [{}]", a.state, a.stamp),
        })
        .collect();
    let mut rewritten = Vec::new();
    let mut all_leftovers = leftovers;
    let mut repeat_incomplete = false;
    if !first.is_empty() && !out_of_time {
        // one repeat on a fresh scratch root; an item written AGAIN in that window is the finding
        let before2 = replay::live_snapshot(lane, &settings);
        if let Some(hook) = &cfg.between_snapshots {
            hook(2);
        }
        // RULINGS CZ6 / EL1: the repeat is complete only when EVERY planned execution ran to a
        // result (computed from the plan) — a spawn failure, one child's time-out, an early return
        // or a failed pass make it incomplete, and an incomplete repeat proves nothing
        let (left2, complete2) = match run_all(cfg, &v, &inputs, until, false) {
            Ok((_again, left2, _oot2, complete2)) => (left2, complete2),
            Err(_) => (Vec::new(), false),
        };
        repeat_incomplete = !complete2;
        let after2 = replay::live_snapshot(lane, &settings);
        all_leftovers.extend(left2);
        rewritten = replay::changed_items(&before2, &after2)
            .into_iter()
            .map(|(b, a)| LiveDifference {
                item: a.item,
                before: format!("{} [{}]", b.state, b.stamp),
                after: format!("{} [{}]", a.state, a.stamp),
            })
            .collect();
    }
    if out_of_time {
        return Err(RepairError::CheckTimeout(format!(
            "보호장치 심사가 시한({}초) 안에 끝나지 않았습니다",
            deadline.as_secs()
        )));
    }
    let still: Vec<u32> = all_leftovers
        .into_iter()
        .filter(|p| replay::group_alive(*p))
        .collect();
    results.sort_by(|a, b| {
        (a.test.as_str(), a.class.as_str(), a.cell.as_str()).cmp(&(
            b.test.as_str(),
            b.class.as_str(),
            b.cell.as_str(),
        ))
    });
    let mut report = summarize(results, fingerprint, started.elapsed());
    report.after_run = AfterRun {
        first,
        rewritten_in_repeat: rewritten,
        leftover_processes: still,
        repeat_incomplete,
    };
    if !report.after_run.leftover_processes.is_empty() {
        report.refusals.push(format!(
            "예행이 끝난 뒤에도 남은 프로세스가 있었습니다(pid {:?}) — 거부합니다",
            report.after_run.leftover_processes
        ));
        report.verdict = ScreenVerdict::Refused;
    }
    Ok(report)
}

/// (results, leftover processes, out of time, the pass COMPLETE — RULINGS EL1).
type RunAll = (Vec<ProbeResult>, Vec<u32>, bool, bool);

fn run_all(
    cfg: &ScreenConfig,
    v: &Variants,
    inputs: &BTreeMap<&'static str, Vec<String>>,
    until: Instant,
    use_cache: bool,
) -> Result<RunAll, RepairError> {
    let scratch = ScratchRoot::new(&cfg.scratch_base, "sc-")?;
    let mut r = Runner::new(cfg, scratch.path(), until)?;
    r.use_cache = use_cache;
    if !use_cache && cfg.fail_first_spawn_in_repeat {
        r.inject_spawn_failure.store(true, Ordering::SeqCst);
    }
    if !use_cache && cfg.repeat_out_of_time {
        r.out_of_time.store(true, Ordering::SeqCst);
    }
    let mut results = Vec::new();
    let empty = Vec::new();
    let g = inputs.get("G").unwrap_or(&empty);
    let gate: Vec<String> = {
        let mut s: BTreeSet<String> = inputs.get("R").unwrap_or(&empty).iter().cloned().collect();
        s.extend(inputs.get("C").unwrap_or(&empty).iter().cloned());
        s.into_iter().collect()
    };
    screen_device(&r, Device::Guard, v, g, &mut results);
    screen_backstop(&r, v, g, &mut results);
    screen_device(&r, Device::Gate, v, &gate, &mut results);
    if cfg.run_i3 {
        screen_i3(&r, v, inputs.get("I3").unwrap_or(&empty), &mut results);
    }
    screen_i4(v, &mut results);
    screen_at4(v, &mut results);
    let leftovers = r.leftovers.lock().map(|l| l.clone()).unwrap_or_default();
    let oot = r.out_of_time.load(Ordering::SeqCst);
    let complete = r.complete();
    drop(r);
    // RULINGS CZ4: a pass whose scratch root cannot be removed is no measured pass
    scratch.close()?;
    Ok((results, leftovers, oot, complete))
}

fn summarize(probes: Vec<ProbeResult>, input_fingerprint: String, took: Duration) -> ScreenReport {
    let mut refusals = Vec::new();
    let mut notes = Vec::new();
    let mut lane_findings = Vec::new();
    let mut not_measurable = Vec::new();
    // refusals grouped by protection (the reason names every protection concerned)
    let mut by_prot: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut findings: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for p in &probes {
        let tag = format!("{} {}", p.class, p.cell);
        match p.decision {
            ProbeDecision::Refused => by_prot.entry(p.protection.as_str()).or_default().push(tag),
            ProbeDecision::LaneFinding => {
                findings.entry(p.protection.as_str()).or_default().push(tag)
            }
            ProbeDecision::NotMeasurable => not_measurable.push(format!("{tag} ({})", p.detail)),
            _ => {}
        }
    }
    for (prot, tags) in &by_prot {
        refusals.push(format!(
            "보호장치 '{prot}': 이 후보를 적용하면 시험 {}건을 통과하지 못하거나 잴 수 없습니다(예: {}) — 거부합니다",
            tags.len(),
            tags.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let n_findings: usize = findings.values().map(Vec::len).sum();
    if n_findings > 0 {
        notes.push(format!(
            "보호장치 시험 {n_findings}건이 이 부서의 지금 파일에서 이미 통과되지 않습니다 — 이 후보 때문이 아닙니다(접수 줄에 남깁니다)"
        ));
        for (prot, tags) in &findings {
            lane_findings.push(format!(
                "보호장치 '{prot}' 시험 {}건이 이 부서의 지금 파일에서 통과되지 않습니다(예: {})",
                tags.len(),
                tags.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if !not_measurable.is_empty() {
        notes.push(format!(
            "보호장치 시험 {}건은 출시본으로 잴 수 없었습니다(후보가 그 장치의 파일을 건드리지 않아 거부 사유는 아닙니다)",
            not_measurable.len()
        ));
    }
    let verdict = if !refusals.is_empty() {
        ScreenVerdict::Refused
    } else if notes.is_empty() {
        ScreenVerdict::Accepted
    } else {
        ScreenVerdict::AcceptedWithNotes
    };
    ScreenReport {
        schema: SCREEN_SCHEMA,
        verdict,
        refusals,
        notes,
        lane_findings,
        not_measurable,
        probes,
        input_fingerprint,
        build_id: screen_build_id(),
        duration_ms: took.as_millis() as u64,
        after_run: AfterRun::default(),
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;

// keep the corpus names used only by the tables reachable from here (census tests)
#[allow(dead_code)]
fn _corpus_names() -> (
    &'static [Cell],
    &'static [(&'static str, Probe, bool, &'static str)],
) {
    (CELLS, BACKSTOP_CLASSES)
}
