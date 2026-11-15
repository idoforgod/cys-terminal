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
/// RELEASED = the copies embedded in this binary. Stub until the body lands (T5b).
pub fn screen_protections(
    candidate: &ScreenCandidate,
    lane: &Path,
    deadline: Duration,
) -> Result<ScreenReport, RepairError> {
    let _ = (candidate, lane, deadline);
    Err(RepairError::NotImplemented("screen::screen_protections"))
}

/// The fingerprint of the screen's input STATES as they will be after the apply (BC7): every file
/// of the generated per-test inputs (G, R, C, I3), the candidate's state for a target, else the
/// lane's live state — `repair::fingerprint` over those (rel, state) lines.
pub fn input_fingerprint(candidate: &ScreenCandidate, lane: &Path) -> Result<String, RepairError> {
    let _ = (candidate, lane);
    Err(RepairError::NotImplemented("screen::input_fingerprint"))
}

/// The screen's build id (BD5): a recorded verdict with another build id is never reused.
pub fn screen_build_id() -> String {
    format!(
        "{}+{}+screen{}",
        crate::pack::build_id(),
        env!("CARGO_PKG_VERSION"),
        SCREEN_SCHEMA
    )
}

/// `<root>/R-<n>/screen.json` — the recorded verdict of a repair (root = the ledger root).
pub fn verdict_path_in(root: &Path, id: &RepairId) -> PathBuf {
    super::ledger::repair_dir_in(root, id).join("screen.json")
}

/// Writes the recorded verdict (durably). Stub until the body lands (T5b).
pub fn write_verdict_in(
    root: &Path,
    id: &RepairId,
    v: &RecordedVerdict,
) -> Result<(), RepairError> {
    let _ = (root, id, v);
    Err(RepairError::NotImplemented("screen::write_verdict_in"))
}

/// Reads the recorded verdict; `Ok(None)` when none was recorded. Stub until the body lands (T5b).
pub fn read_verdict_in(root: &Path, id: &RepairId) -> Result<Option<RecordedVerdict>, RepairError> {
    let _ = (root, id);
    Err(RepairError::NotImplemented("screen::read_verdict_in"))
}

/// AY5 at apply: the recorded verdict holds — nothing runs — when it is not a refusal, its build id
/// is this binary's [`screen_build_id`] and its fingerprint equals [`input_fingerprint`] of the
/// candidate in `lane`. Stub until the body lands (T5b).
pub fn recorded_verdict_holds(
    recorded: &RecordedVerdict,
    candidate: &ScreenCandidate,
    lane: &Path,
) -> Result<bool, RepairError> {
    let _ = (recorded, candidate, lane);
    Err(RepairError::NotImplemented(
        "screen::recorded_verdict_holds",
    ))
}
