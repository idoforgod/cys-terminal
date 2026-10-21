// Producers (queue, direct send, schedule/channel/feed/boot notices) are wired by the following S-a
// commits; until then only the writer arm (`state::run_writer_loop_tracked`) and tests build jobs.
#![allow(dead_code)]
//! S-a checked agent writer (DESIGN-S-FROZEN-R3 §4.4-4.5, §4.7; HEAD-R3-RULING S4-1).
//!
//! Every machine write aimed at an agent receiver is ONE [`AgentJob`] executed by the surface's
//! single writer thread (arm `WriteReq::Agent`). Inside a job the order is fixed: durable `writing`
//! journal record -> full admission proof -> OS calls. Before EVERY later OS call — including a
//! short-write or EINTR retry, and after every wait (clear settle, paste-to-Return spacing, submit
//! gap) — the job checks its cancellation flag and re-proves the receiver; each call offers at most
//! [`MAX_CALL_BYTES`]. The terminal outcome is journaled before the producer's settlement closure
//! runs, so no producer can pop, count or announce delivery before the PTY accepted the bytes.
//! Refusal, cancellation and partial results are typed outcomes: they never end the writer loop.
//!
//! S-a residual (stated, not solved): a process can die after a passing check and before the next
//! OS call; bytes already accepted can still be read by the surviving shell; an issued native call
//! is never interrupted. Containment and native cancellation are S-b.

use crate::receiver::ProofLoss;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Per-OS-call cap (design 4.4 step 3).
pub const MAX_CALL_BYTES: usize = 4096;
const FENCE_OPEN: &[u8] = cys::paste_fence::OPEN.as_bytes();
const FENCE_CLOSE: &[u8] = cys::paste_fence::CLOSE.as_bytes();
const CLEAR: &[u8] = b"\x15";
const RETURN: &[u8] = b"\r";

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// Daemon-unique request id (`<daemon start ms>-<seq>` keeps ids distinct across restarts).
pub fn new_request_id(daemon_started_ms: u64) -> String {
    format!("w{daemon_started_ms:x}-{}", NEXT_REQUEST.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Clear,
    OpenFence,
    Body,
    CloseFence,
    Control,
    Submit,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::OpenFence => "open_fence",
            Self::Body => "body",
            Self::CloseFence => "close_fence",
            Self::Control => "control",
            Self::Submit => "submit",
        }
    }
}

/// The request's own Return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submit {
    /// Paste-to-Return spacing after this request's body (the existing `cr_delay_ms`).
    pub delay_ms: u64,
    /// Minimum gap since this writer's last program write (`SubmitAfterGap` semantics; 0 = off).
    pub min_gap_ms: u64,
}

/// What one request writes, in order. Built only through the constructors so a fenced body is
/// sanitized exactly once with the existing paste-fence rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    clear_first: bool,
    clear_settle_ms: u64,
    body: Option<String>,
    fenced: bool,
    control: Option<Vec<u8>>,
    submit: Option<Submit>,
}

/// Exact private borrowed projection for durable codecs; no default serialization or Debug.
/// It does not reconstruct an executable plan or authorize writes from recovered data.
pub struct PlanView<'a> {
    pub clear_first:bool,pub clear_settle_ms:u64,pub body:Option<&'a str>,pub fenced:bool,
    pub control:Option<&'a [u8]>,pub submit:Option<Submit>,
}
impl Plan {
    pub fn exact_view(&self)->PlanView<'_> {
        PlanView { clear_first:self.clear_first,clear_settle_ms:self.clear_settle_ms,body:self.body.as_deref(),
            fenced:self.fenced,control:self.control.as_deref(),submit:self.submit }
    }
    fn empty() -> Plan {
        Plan { clear_first: false, clear_settle_ms: 0, body: None, fenced: false, control: None, submit: None }
    }
    /// One bracketed-paste body: open fence + body + close fence are byte-identical to
    /// `cys::paste_fence::wrap(text)` (the existing `WriteReq::Inject` bytes).
    pub fn paste(text: &str) -> Plan {
        Plan { body: Some(cys::paste_fence::sanitize(text).into_owned()), fenced: true, ..Plan::empty() }
    }
    /// An unfenced program body (the direct-send `DirectFence::Raw` mapping): bytes as given.
    pub fn raw(text: &str) -> Plan {
        Plan { body: Some(text.to_string()), ..Plan::empty() }
    }
    /// Control bytes instead of a body (AgentControl: Ctrl-U, Escape, a dialog digit ...).
    pub fn control(bytes: Vec<u8>) -> Plan {
        Plan { control: Some(bytes), ..Plan::empty() }
    }
    /// A Return that is this request's only phase (bound standalone submit, design 4.5).
    pub fn submit_only(min_gap_ms: u64) -> Plan {
        Plan { submit: Some(Submit { delay_ms: 0, min_gap_ms }), ..Plan::empty() }
    }
    /// Ctrl-U before the body, then `settle_ms` (the existing clear-first settle).
    pub fn with_clear(mut self, settle_ms: u64) -> Plan {
        self.clear_first = true;
        self.clear_settle_ms = settle_ms;
        self
    }
    /// The request owns its Return, written `delay_ms` after the body.
    pub fn with_submit(mut self, delay_ms: u64, min_gap_ms: u64) -> Plan {
        self.submit = Some(Submit { delay_ms, min_gap_ms });
        self
    }
    /// Data-only append; the existing Request still requires an actual durable edit grant.
    pub(crate) fn append_owned_submit(&self,submit:Submit)->Result<Plan,LeaseError> {
        if self.submit.is_some() || self.body.is_none() || self.control.is_some() {
            return Err(LeaseError::PlanChanged);
        }
        let mut next=self.clone();next.submit=Some(submit);Ok(next)
    }
    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }
    pub fn owns_submit(&self) -> bool {
        self.submit.is_some()
    }
    /// Every byte the plan writes, in order.
    pub fn planned_bytes(&self) -> Vec<u8> {
        calls_for(self, &Envelope::default()).into_iter().flat_map(|(_, b)| b).collect()
    }
}

/// Where a request is, byte-exactly (S4-1 PartialExact). Cumulative over resumed runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    pub cleared: bool,
    pub opened: bool,
    /// Body bytes accepted so far.
    pub body_offset: usize,
    pub closed: bool,
    pub control_done: bool,
    pub submitted: bool,
}

/// Accepted byte counts of ONE run, per column (payload / fence / control, design 4.4); `calls`
/// counts OS write attempts including EINTR retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Accepted {
    pub payload: u64,
    pub fence: u64,
    pub control: u64,
    pub calls: u64,
}

impl Accepted {
    pub fn total(&self) -> u64 {
        self.payload + self.fence + self.control
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminal {
    /// Journal, proof or transport refused before the first byte: ZERO bytes (incl. fence/Ctrl-U/Return).
    RefusedNoWrite { reason: String },
    CancelledBeforeWrite,
    /// Body (or control) complete; the request owns no submit phase.
    WrittenBody { accepted: Accepted },
    /// Every phase including the request's own Return complete.
    WrittenSubmitted { accepted: Accepted },
    /// Stopped by a resumable (non-death, non-replacement) proof loss with no call outstanding at a
    /// byte-exact point: the SAME request may resume from `envelope` (S4-1).
    PartialExact { envelope: Envelope, accepted: Accepted, reason: String },
    /// Stopped after bytes by death/replacement, transport error, cancellation or a non-exact cut:
    /// never replayed (design 4.6 reconciliation).
    PartialUnknown { envelope: Envelope, accepted: Accepted, reason: String },
}

impl Terminal {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RefusedNoWrite { .. } => "refused_no_write",
            Self::CancelledBeforeWrite => "cancelled_before_write",
            Self::WrittenBody { .. } => "written_body",
            Self::WrittenSubmitted { .. } => "written_submitted",
            Self::PartialExact { .. } => "partial_exact",
            Self::PartialUnknown { .. } => "partial_unknown",
        }
    }
    pub fn wrote_nothing(&self) -> bool {
        matches!(self, Self::RefusedNoWrite { .. } | Self::CancelledBeforeWrite)
    }
}

/// Lifecycle of one request as seen by status readers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    AcceptedPending,
    Writing,
    Terminal(Terminal),
}

/// Shared record of one logical request (status reads never block on the writer).
pub struct Request {
    pub id: String,
    pub surface_id: u64,
    pub origin: &'static str,
    pub digest: String,
    pub created: Instant,
    cancelled: AtomicBool,
    status: Mutex<Status>,
    /// In-process completion ended, including a caught callback panic. Not durable source settlement.
    settled: AtomicBool,
    /// When the receiver proof was last lost (admission or recheck): the S4-1 resume window counts from here.
    lost_at: Mutex<Option<Instant>>,
    native: Mutex<NativeState>,
    managed: bool,
}

impl Request {
    pub fn new(id: String, surface_id: u64, origin: &'static str, digest: String) -> Arc<Request> {
        Self::new_mode(id, surface_id, origin, digest, Instant::now(), None)
    }
    pub(crate) fn new_managed(id: String, surface_id: u64, origin: &'static str, digest: String, plan: &Plan) -> Arc<Request> {
        Self::new_managed_at(id, surface_id, origin, digest, Instant::now(), plan)
    }
    pub(crate) fn new_managed_at(id: String, surface_id: u64, origin: &'static str, digest: String, received_at: Instant, plan: &Plan) -> Arc<Request> {
        Self::new_mode(id, surface_id, origin, digest, received_at, Some(plan))
    }
    fn new_mode(id: String, surface_id: u64, origin: &'static str, digest: String, received_at: Instant, admitted_plan: Option<&Plan>) -> Arc<Request> {
        Arc::new(Request {
            id,
            surface_id,
            origin,
            digest,
            created: received_at,
            cancelled: AtomicBool::new(false),
            status: Mutex::new(Status::AcceptedPending),
            settled: AtomicBool::new(false),
            lost_at: Mutex::new(None),
            native: Mutex::new(NativeState { plan: admitted_plan.cloned(), ..NativeState::default() }),
            managed: admitted_plan.is_some(),
        })
    }
    /// The terminal is recorded and in-process settlement ended; this is not a durable source receipt.
    pub fn is_settled(&self) -> bool {
        self.settled.load(Ordering::Acquire)
    }
    /// Request cancellation: revokes LATER calls only; an issued call is never interrupted (S-a).
    pub fn cancel(&self) {
        // Managed edit commit and cancellation have one linearization mutex.
        // No native mutex is held across a disk operation, proof or native call.
        if self.managed {
            let _state = self.native.lock().unwrap_or_else(|e| e.into_inner());
            self.cancelled.store(true, Ordering::Release);
        } else {
            self.cancelled.store(true, Ordering::Release);
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// The instant of the latest receiver-proof loss of this request (`None` = never lost).
    pub fn lost_at(&self) -> Option<Instant> {
        *self.lost_at.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn mark_lost(&self) {
        *self.lost_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
    fn set(&self, s: Status) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
}

/// Exact writer-owned facts supplied to receiver proofs; no parser/last-body reconstruction.
pub struct ProofContext<'a> {
    pub request: &'a Request,
    pub plan: &'a Plan,
    pub envelope: Envelope,
    pub accepted: Accepted,
    pub resumed: bool,
    pub phase_offset: usize,
    pub next_phase: Option<Phase>,
}

/// Request-lifetime native progress. Unknown completion is sticky across later exact results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct NativeProgress {
    pub accepted_total: u64,
    pub outstanding: bool,
    pub unknown: bool,
    pub envelope: Envelope,
    pub run_accepted: Accepted,
}

impl NativeProgress {
    pub fn proven_zero(&self) -> bool {
        self.accepted_total == 0 && self.envelope == Envelope::default() && !self.outstanding && !self.unknown
    }
}

#[derive(Default)]
struct NativeState {
    progress: NativeProgress,
    plan: Option<Plan>,
    running: bool,
    resume: Option<Envelope>,
    final_outcome: bool,
    completion_key: Option<Arc<()>>,
    settling: bool,
    operation: Option<ManagedOperation>,
    // Acknowledge only this actual zero-release again, until any new ownership.
    // Runtime identity only: never serialized or inferred from absence/zero bytes.
    last_zero_resolution: Option<Arc<()>>,
    // Positive history of the newest issued managed owner, including after its lease
    // is dropped/completed. One bounded nonce, never persisted or a release permit.
    last_issued_owner: Option<Arc<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaseError {
    Unmanaged, Cancelled, Busy, NativeConflict, WrongRequest, WrongLease,
    AlreadyStarted, PlanChanged, ResumeChanged, EditNotPersisting, ResolutionMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptPhase { Prepared, Started }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditPhase { Reserved, Persisting }

enum ManagedOperation {
    Attempt { key: Arc<()>, phase: AttemptPhase },
    Edit { key: Arc<()>, phase: EditPhase },
}

/// Non-Clone capability carried by the actual queued job. No Request-id comparison.
pub(crate) struct AttemptLease {
    request: Arc<Request>, key: Arc<()>, plan: Plan, resume: Option<Envelope>,
}

/// Opaque in-memory association identity; not serializable and not a completion proof.
/// A restarted daemon must reconstruct authority through its durable recovery protocol.
#[derive(Clone)]
pub(crate) struct AttemptStamp {
    request: Arc<Request>, key: Arc<()>, plan: Plan, resume: Option<Envelope>,
}

/// Read-only relationship to the exact private stamp, never a native/store permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttemptOwnership { Same, Superseded }
impl AttemptStamp {
    pub(crate) fn ownership(&self, request:&Arc<Request>)->Result<AttemptOwnership,LeaseError> {
        if !request.managed || !Arc::ptr_eq(request,&self.request) {
            return Err(LeaseError::WrongRequest);
        }
        let state=request.native.lock().unwrap_or_else(|e|e.into_inner());
        let latest=state.last_issued_owner.as_ref().ok_or(LeaseError::WrongLease)?;
        Ok(if Arc::ptr_eq(latest,&self.key) { AttemptOwnership::Same }
            else { AttemptOwnership::Superseded })
    }
}

/// Exclusive zero-byte edit ownership. No native mutex survives a public method.
pub(crate) struct ZeroWriteEditLease {
    request: Arc<Request>, key: Arc<()>, original_plan: Option<Plan>,
}

/// An edit's opaque identity and exact pre-edit plan. Possession alone never means
/// the disk CAS committed, submit ownership exists, or lineage is authorized.
#[derive(Clone)]
pub(crate) struct EditStamp {
    request: Arc<Request>, key: Arc<()>, original_plan: Option<Plan>,
}

/// No public constructor/Clone. Selection-specific issuance is bound to the actual
/// pending store CAS by the typed bridge below; no generic public issuer exists.
/// Tests may construct these module-private fields as explicit synthetic receipts.
struct DurableEditCommit { stamp: EditStamp }
struct DurableEditNoChange { stamp: EditStamp }
enum DurableEditResolution {
    Committed(DurableEditCommit),
    NotWritten(DurableEditNoChange),
}

impl NativeState {
    /// Existing begin_run rule shared by reservation and execution. This does not
    /// calculate byte progress; NativeCall remains its only native-I/O producer.
    fn permits_run(&self, plan: &Plan, resume: Option<Envelope>) -> bool {
        let p = self.progress;
        let permitted = match resume {
            Some(env) => self.resume == Some(env) && env == p.envelope,
            None => self.resume.is_none() && p.proven_zero(),
        };
        !self.running && !self.settling && !self.final_outcome
            && !p.outstanding && !p.unknown && permitted
            && self.plan.as_ref().is_none_or(|original| original == plan)
    }

    fn permits_zero_edit(&self) -> bool {
        self.plan.is_some() && self.progress.proven_zero() && !self.running && !self.settling
            && !self.final_outcome && self.resume.is_none() && self.completion_key.is_none()
    }

    /// The only way a new managed owner is issued: it becomes the newest owner and the
    /// previous run's zero-release acknowledgement ends, in the same native-mutex hold.
    fn issue_owner(&mut self, key: &Arc<()>, operation: ManagedOperation) {
        self.last_zero_resolution = None;
        self.last_issued_owner = Some(key.clone());
        self.operation = Some(operation);
    }
}

impl Request {
    /// Reserve BEFORE Prepared is published or the job is queued. The lease binds
    /// the actual object, exact plan, and exact starting resume envelope.
    pub(crate) fn reserve_attempt(
        self: &Arc<Self>, plan: &Plan, resume: Option<Envelope>,
    ) -> Result<AttemptLease, LeaseError> {
        if !self.managed { return Err(LeaseError::Unmanaged); }
        let mut state = self.native.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() { return Err(LeaseError::Cancelled); }
        if state.operation.is_some() { return Err(LeaseError::Busy); }
        if !state.permits_run(plan, resume) { return Err(LeaseError::NativeConflict); }
        let key = Arc::new(());
        state.issue_owner(&key, ManagedOperation::Attempt { key: key.clone(), phase: AttemptPhase::Prepared });
        Ok(AttemptLease { request: self.clone(), key, plan: plan.clone(), resume })
    }

    pub(crate) fn reserve_zero_write_edit(self: &Arc<Self>) -> Result<ZeroWriteEditLease, LeaseError> {
        if !self.managed { return Err(LeaseError::Unmanaged); }
        let mut state = self.native.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() { return Err(LeaseError::Cancelled); }
        if state.operation.is_some() { return Err(LeaseError::Busy); }
        if !state.permits_zero_edit() { return Err(LeaseError::NativeConflict); }
        let key = Arc::new(());
        let original_plan = state.plan.clone();
        state.issue_owner(&key, ManagedOperation::Edit { key: key.clone(), phase: EditPhase::Reserved });
        Ok(ZeroWriteEditLease { request: self.clone(), key, original_plan })
    }
}

impl AttemptLease {
    /// Pure validation for the pending protocol's in-memory association. No disk
    /// result and no authority for submit/lineage is inferred by returning a stamp.
    pub(crate) fn validate_prepared(
        &self, request: &Arc<Request>, plan: &Plan, resume: Option<Envelope>,
    ) -> Result<AttemptStamp, LeaseError> {
        if !Arc::ptr_eq(request, &self.request) { return Err(LeaseError::WrongRequest); }
        if plan != &self.plan { return Err(LeaseError::PlanChanged); }
        if resume != self.resume { return Err(LeaseError::ResumeChanged); }
        let state = request.native.lock().unwrap_or_else(|e| e.into_inner());
        if request.is_cancelled() { return Err(LeaseError::Cancelled); }
        match &state.operation {
            Some(ManagedOperation::Attempt { key, phase: AttemptPhase::Prepared })
                if Arc::ptr_eq(key, &self.key) && state.permits_run(plan, resume) =>
                    Ok(AttemptStamp { request: request.clone(), key: self.key.clone(), plan: self.plan.clone(), resume: self.resume }),
            Some(ManagedOperation::Attempt { key, phase: AttemptPhase::Started })
                if Arc::ptr_eq(key, &self.key) => Err(LeaseError::AlreadyStarted),
            _ => Err(LeaseError::WrongLease),
        }
    }
}

impl Drop for AttemptLease {
    fn drop(&mut self) {
        let mut state = self.request.native.lock().unwrap_or_else(|e| e.into_inner());
        let own_unstarted = matches!(&state.operation,
            Some(ManagedOperation::Attempt { key, phase: AttemptPhase::Prepared })
                if Arc::ptr_eq(key, &self.key));
        if own_unstarted && !state.running && !state.settling
            && !state.progress.outstanding && !state.progress.unknown {
            state.operation = None;
        }
        // Started/uncertain ownership survives a dropped queue/job/handle. Only
        // real finish or an applicable typed reconciliation receipt can resolve it.
    }
}

impl ZeroWriteEditLease {
    /// Read-only ownership check for a held pending-protocol adapter. Success is
    /// only this native exclusion relation; it is not durable CAS authorization.
    pub(crate) fn validate_held(&self, request: &Arc<Request>, plan: &Plan) -> Result<(), LeaseError> {
        self.validate_phase(request,plan,None)
    }
    /// Initial metadata mutation cannot accept an already unresolved persistence operation.
    pub(crate) fn validate_reserved(&self,request:&Arc<Request>,plan:&Plan)->Result<(),LeaseError> {
        self.validate_phase(request,plan,Some(EditPhase::Reserved))
    }
    fn validate_phase(&self,request:&Arc<Request>,plan:&Plan,required:Option<EditPhase>)->Result<(),LeaseError> {
        if !Arc::ptr_eq(request, &self.request) { return Err(LeaseError::WrongRequest); }
        if self.original_plan.as_ref() != Some(plan) { return Err(LeaseError::PlanChanged); }
        let state = request.native.lock().unwrap_or_else(|e| e.into_inner());
        if request.is_cancelled() { return Err(LeaseError::Cancelled); }
        if !state.permits_zero_edit() || state.plan != self.original_plan {
            return Err(LeaseError::NativeConflict);
        }
        match &state.operation {
            Some(ManagedOperation::Edit { key, phase }) if Arc::ptr_eq(key, &self.key)
                && required.is_none_or(|wanted|wanted==*phase) => Ok(()),
            _ => Err(LeaseError::WrongLease),
        }
    }

    fn stamp(&self) -> EditStamp {
        EditStamp { request: self.request.clone(), key: self.key.clone(), original_plan: self.original_plan.clone() }
    }

    /// Enter before starting disk/CAS work; this method returns with NO mutex held.
    /// Once persistence may have started, Drop cannot assume rollback/no change.
    pub(crate) fn begin_persisting(&mut self, plan: &Plan) -> Result<EditStamp, LeaseError> {
        let mut state = self.request.native.lock().unwrap_or_else(|e| e.into_inner());
        if self.request.is_cancelled() { return Err(LeaseError::Cancelled); }
        if self.original_plan.as_ref() != Some(plan) { return Err(LeaseError::PlanChanged); }
        if !state.permits_zero_edit() || state.plan != self.original_plan {
            return Err(LeaseError::NativeConflict);
        }
        match &mut state.operation {
            Some(ManagedOperation::Edit { key, phase })
                if Arc::ptr_eq(key, &self.key) && *phase == EditPhase::Reserved => {
                    *phase = EditPhase::Persisting;
                    Ok(self.stamp())
                }
            _ => Err(LeaseError::WrongLease),
        }
    }

    /// Commit/rollback ONLY from a non-forgeable durable-domain resolution. This
    /// resolution is issued by the actual pending-store selection transaction.
    fn resolve(self, resolution: DurableEditResolution) -> Result<(), LeaseError> {
        let (stamp, committed) = match resolution {
            DurableEditResolution::Committed(receipt) => (receipt.stamp, true),
            DurableEditResolution::NotWritten(receipt) => (receipt.stamp, false),
        };
        if !Arc::ptr_eq(&stamp.request, &self.request) || !Arc::ptr_eq(&stamp.key, &self.key)
            || stamp.original_plan != self.original_plan {
            return Err(LeaseError::ResolutionMismatch);
        }
        let mut state = self.request.native.lock().unwrap_or_else(|e| e.into_inner());
        let own = matches!(&state.operation,
            Some(ManagedOperation::Edit { key, phase: EditPhase::Persisting }) if Arc::ptr_eq(key, &self.key));
        if !own { return Err(LeaseError::EditNotPersisting); }
        if !state.permits_zero_edit() || state.plan != self.original_plan {
            return Err(LeaseError::NativeConflict);
        }
        if committed && self.request.is_cancelled() {
            // Disk may have committed while cancellation arrived. Keep the
            // reservation for durable reconciliation; never revive this request.
            return Err(LeaseError::Cancelled);
        }
        state.operation = None;
        // This slice NEVER changes the admitted Plan. It only resolves an edit to
        // durable association/lineage metadata under the separate domain token.
        // No final_outcome, native counts, resume permission, cancellation, submit
        // slot, binding/conversation epoch, body lineage or durable ID is reset.
        Ok(())
    }
}

impl Drop for ZeroWriteEditLease {
    fn drop(&mut self) {
        let mut state = self.request.native.lock().unwrap_or_else(|e| e.into_inner());
        let own_reserved = matches!(&state.operation,
            Some(ManagedOperation::Edit { key, phase: EditPhase::Reserved }) if Arc::ptr_eq(key, &self.key));
        if own_reserved && state.permits_zero_edit() {
            state.operation = None;
        }
        // Persisting is not rolled back by timeout/drop; disk outcome is unknown.
    }
}

/// Writer-created only after an actual managed lease start. This is not a public
/// ProofContext constructor or an assertion that a durable pending CAS succeeded.
pub struct AttemptStartReceipt<'a> {
    request: &'a Arc<Request>, key: &'a Arc<()>, plan: &'a Plan, resume: Option<Envelope>,
}
pub(crate) struct StartedAttempt<'r, 'c> { start: &'r AttemptStartReceipt<'c> }
impl<'c> AttemptStartReceipt<'c> {
    pub(crate) fn for_attempt<'r>(&'r self, stamp: &AttemptStamp) -> Result<StartedAttempt<'r, 'c>, LeaseError> {
        if !self.request.managed || !Arc::ptr_eq(self.request, &stamp.request)
            || !Arc::ptr_eq(self.key, &stamp.key) || self.plan != &stamp.plan || self.resume != stamp.resume {
            return Err(LeaseError::WrongLease);
        }
        Ok(StartedAttempt { start: self })
    }
}
impl StartedAttempt<'_, '_> {
    pub(crate) fn request(&self) -> &Request { self.start.request }
    pub(crate) fn plan(&self) -> &Plan { self.start.plan }
    pub(crate) fn resume(&self) -> Option<Envelope> { self.start.resume }
}

/// A private completion tied to the exact Prepared association. It is still only
/// writer provenance, not proof that the producer's durable transaction closed.
pub(crate) struct AttemptReceipt<'r, 'c> { receipt: &'r WriterReceipt<'c> }
impl<'c> WriterReceipt<'c> {
    pub(crate) fn for_attempt<'r>(&'r self, stamp: &AttemptStamp) -> Result<AttemptReceipt<'r, 'c>, LeaseError> {
        if !self.completed.request.managed || !Arc::ptr_eq(&stamp.request, &self.completed.request)
            || !Arc::ptr_eq(&stamp.key, &self.completed.key) || stamp.plan != self.completed.plan
            || stamp.resume.is_some() != self.completed.resumed {
            return Err(LeaseError::WrongLease);
        }
        Ok(AttemptReceipt { receipt: self })
    }
}
impl<'r, 'c> AttemptReceipt<'r, 'c> {
    pub(crate) fn writer(&self) -> &WriterReceipt<'c> { self.receipt }
}

impl Request {
    pub fn native_progress(&self) -> NativeProgress {
        self.native.lock().unwrap_or_else(|e| e.into_inner()).progress
    }
    fn begin_run(
        self: &Arc<Self>, plan: &Plan, resume: Option<Envelope>, attempt: Option<&AttemptLease>,
    ) -> Result<Arc<()>, NativeProgress> {
        let mut state=self.native.lock().unwrap_or_else(|e| e.into_inner());
        let p=state.progress;
        if !state.permits_run(plan,resume) { return Err(p); }
        let key=if self.managed {
            let Some(lease)=attempt else { return Err(p); };
            if !Arc::ptr_eq(self,&lease.request) || &lease.plan!=plan || lease.resume!=resume {
                return Err(p);
            }
            match &mut state.operation {
                Some(ManagedOperation::Attempt { key, phase })
                    if Arc::ptr_eq(key,&lease.key) && *phase==AttemptPhase::Prepared => {
                        *phase=AttemptPhase::Started;
                        key.clone() // reuse reservation identity as existing completion identity
                    }
                _=>return Err(p),
            }
        } else {
            if attempt.is_some() || state.operation.is_some() { return Err(p); }
            Arc::new(()) // legacy Request::new behavior remains unchanged
        };
        if state.plan.is_none() { state.plan=Some(plan.clone()); }
        state.running=true;
        state.resume=None;
        state.progress.run_accepted=Accepted::default();
        state.completion_key=Some(key.clone());
        self.settled.store(false, Ordering::Release);
        Ok(key)
    }
    fn end_run(&self, key:&Arc<()>, terminal: &Terminal)->Option<NativeProgress> {
        let mut state=self.native.lock().unwrap_or_else(|e| e.into_inner());
        if !state.completion_key.as_ref().is_some_and(|current|Arc::ptr_eq(current,key)) { return None; }
        state.running=false;
        state.resume=match terminal { Terminal::PartialExact{envelope,..}=>Some(*envelope),_=>None };
        state.final_outcome=!matches!(terminal,Terminal::RefusedNoWrite{..}|Terminal::PartialExact{..});
        Some(state.progress)
    }
    fn claim_completion(&self,key:&Arc<()>)->bool {
        let mut state=self.native.lock().unwrap_or_else(|e|e.into_inner());
        if state.running || state.settling || !state.completion_key.as_ref().is_some_and(|current|Arc::ptr_eq(current,key)) { return false; }
        state.settling=true;true
    }
    fn native_call(&self, envelope: Envelope, accepted: Accepted, write: bool) -> NativeCall<'_> {
        let mut state=self.native.lock().unwrap_or_else(|e| e.into_inner());
        let p=&mut state.progress;
        p.outstanding=true;
        p.envelope=envelope;
        p.run_accepted=accepted;
        if write { p.run_accepted.calls+=1; }
        NativeCall { request:self, completed:false }
    }
}

/// Never holds a mutex across native I/O. A panic leaves an unknown result, not an invented zero.
struct NativeCall<'a> { request: &'a Request, completed: bool }
impl NativeCall<'_> {
    fn complete(mut self, n: usize, envelope: Envelope, accepted: Accepted) {
        let mut state=self.request.native.lock().unwrap_or_else(|e| e.into_inner());
        let p=&mut state.progress;
        p.accepted_total=match p.accepted_total.checked_add(n as u64) {
            Some(total)=>total,
            None=>{p.unknown=true;u64::MAX}
        };
        p.envelope=envelope;
        p.run_accepted=accepted;
        p.outstanding=false;
        self.completed=true;
    }
}
impl Drop for NativeCall<'_> {
    fn drop(&mut self) {
        if !self.completed {
            let mut state=self.request.native.lock().unwrap_or_else(|e| e.into_inner());
            state.progress.unknown=true;
            state.progress.outstanding=false;
        }
    }
}

/// The receiver proof the writer consults. `admit` runs once, after the journal barrier and before
/// the first byte (binding + current input state); `recheck` runs before every later OS call.
pub trait Proof: Send + Sync {
    fn admit(&self) -> Result<(), ProofLoss>;
    fn recheck(&self, phase: Phase) -> Result<(), ProofLoss>;
    fn admit_context(&self, _ctx: &ProofContext<'_>) -> Result<(), ProofLoss> { self.admit() }
    fn recheck_context(&self, phase: Phase, _ctx: &ProofContext<'_>) -> Result<(), ProofLoss> { self.recheck(phase) }
}

/// Durable request journal (design 4.4 step 2 / 4.5): `writing` must be durable before any byte;
/// `terminal` is written before settlement runs.
pub trait Journal: Send + Sync {
    fn writing(&self, req: &Request) -> std::io::Result<()>;
    fn terminal(&self, req: &Request, t: &Terminal, envelope: &Envelope) -> std::io::Result<()>;
    /// Actual writer context, never a constructor-captured approximation of the job's plan.
    fn writing_context(&self,ctx:&ProofContext<'_>)->std::io::Result<()> { self.writing(ctx.request) }
    /// Default preserves legacy journals. Managed adapters must require/validate
    /// the private start receipt against their Prepared AttemptStamp before CAS.
    fn writing_attempt(&self,ctx:&ProofContext<'_>,_attempt:Option<&AttemptStartReceipt<'_>>)->std::io::Result<()> {
        self.writing_context(ctx)
    }
    /// Only finish constructs this receipt after claiming the exact completed run.
    fn terminal_receipt(&self,receipt:&WriterReceipt<'_>)->std::io::Result<()> {
        self.terminal(receipt.request(),receipt.terminal(),&receipt.envelope())
    }
}

/// Read-only actual completion facts. No public constructor, Clone or deserialization path.
/// This is writer provenance, not proof of durable source/association settlement.
pub struct WriterReceipt<'a> { completed:&'a CompletedRun }
impl WriterReceipt<'_> {
    pub fn request(&self)->&Request { &self.completed.request }
    pub fn plan(&self)->&Plan { &self.completed.plan }
    pub fn terminal(&self)->&Terminal { &self.completed.terminal }
    pub fn envelope(&self)->Envelope { self.completed.envelope }
    pub fn native_progress(&self)->NativeProgress { self.completed.native }
    pub fn resumed(&self)->bool { self.completed.resumed }
    /// Writer-stage facts only. The actual durable store remains authoritative.
    pub fn writing_confirmed(&self)->bool { self.completed.journaled }
    pub fn writing_uncertain(&self)->bool { self.completed.journal_uncertain }
}

pub type Settle = Box<dyn FnOnce(&Terminal) + Send>;

pub struct AgentJob {
    pub request: Arc<Request>,
    /// Required for Request::new_managed; reserve before Prepared/queue admission.
    /// All existing legacy AgentJob literals must add attempt: None on migration.
    pub(crate) attempt: Option<AttemptLease>,
    pub plan: Plan,
    pub proof: Arc<dyn Proof>,
    pub journal: Option<Arc<dyn Journal>>,
    /// Resume an earlier PartialExact of the SAME request from this envelope (S4-1).
    pub resume: Option<Envelope>,
    pub settle: Option<Settle>,
    /// The queue's hand-off guard (`state::InjectGuard`): claimed after the journal barrier and
    /// before admission, so its safety probe stays next to the first byte (a failed barrier claims
    /// it too, handing that refusal to the settlement). A lost claim is [`GUARD_ABORTED`] with zero
    /// bytes; the handing-off caller owns that rollback.
    pub guard: Option<Arc<crate::state::InjectGuard>>,
    /// Record the written body as a machine body (`InjectTrack::note_body`): the queue's daemon-made
    /// bodies always (without an owner, like the `Inject` arm); a direct send only with an owner grade
    /// (the old direct path never recorded owner-token / GUI-grade text as a machine body).
    pub note_body: bool,
    /// The owner grade recorded with that body.
    pub body_owner: Option<crate::governance::InputOwner>,
}

/// RefusedNoWrite reason when the hand-off guard was not claimed (the caller already rolled back).
pub const GUARD_ABORTED: &str = "guard_aborted";

/// Writer-thread state a job reads and updates.
pub struct WriterIo<'a> {
    pub sleep: &'a dyn Fn(Duration),
    /// The writer's B2' anchor (`run_writer_loop_tracked`): stamped like `Program` for a body
    /// without its own Return and like `SubmitAfterGap` for a Return-only request; a body with
    /// its own Return does not stamp (like `Inject`).
    pub last_program_write: &'a mut Option<Instant>,
    /// Called once, immediately before the first OS write of this run.
    pub on_first_byte: &'a mut dyn FnMut(),
    /// Called once with the whole body when its last byte is accepted - before the closing fence,
    /// control or Return - so a later phase's proof (the CR withhold probe) sees this body recorded.
    pub on_body_written: &'a mut dyn FnMut(&str),
}

/// Result of [`execute`]: the terminal, where the request stands, and whether the PTY transport
/// broke (the only case in which the writer loop stops).
pub struct Executed {
    pub terminal: Terminal,
    pub envelope: Envelope,
    pub transport_broken: bool,
    /// A `writing` record was made durable, so a terminal record is owed.
    pub journaled: bool,
    completed: Option<CompletedRun>,
}
struct CompletedRun {
    request:Arc<Request>,key:Arc<()>,plan:Plan,terminal:Terminal,envelope:Envelope,
    native:NativeProgress,resumed:bool,journaled:bool,journal_uncertain:bool,terminal_owed:bool,journal:Option<Arc<dyn Journal>>,
}
struct Completing<'a> { request:&'a Request,key:&'a Arc<()>,release_attempt:bool }
impl Drop for Completing<'_> {
    fn drop(&mut self) {
        let mut state=self.request.native.lock().unwrap_or_else(|e|e.into_inner());
        if state.completion_key.as_ref().is_some_and(|current|Arc::ptr_eq(current,self.key)) {
            state.completion_key=None;state.settling=false;
            let own_attempt=matches!(&state.operation,
                Some(ManagedOperation::Attempt { key, phase:AttemptPhase::Started })
                    if Arc::ptr_eq(key,self.key));
            if own_attempt && self.release_attempt && !state.progress.outstanding && !state.progress.unknown {
                // Actual finish already released this exact zero-progress attempt. Its real
                // stored-zero receipt may acknowledge that release until new ownership begins.
                if state.progress.proven_zero() { state.last_zero_resolution=Some(self.key.clone()); }
                state.operation=None;
            }
            // Native uncertainty remains held for typed reconciliation even after
            // a PartialUnknown terminal; dropping the AgentJob never frees it.
        }
    }
}

impl Executed {
    fn unowned(progress:NativeProgress,reason:&str)->Self {
        Self { terminal:Terminal::PartialUnknown { envelope:progress.envelope,accepted:Accepted::default(),reason:reason.into() },
            envelope:progress.envelope,transport_broken:false,journaled:false,completed:None }
    }
    /// The key was captured BEFORE this run started; never recover another run's current key.
    fn panicked(job: &AgentJob,key:Arc<()>) -> Executed {
        let progress=job.request.native_progress();
        let terminal=Terminal::PartialUnknown { envelope:progress.envelope,
            accepted:progress.run_accepted, reason:"writer_panic".into() };
        let Some(native)=job.request.end_run(&key,&terminal) else { return Self::unowned(progress,"panic_owner_conflict"); };
        Self { terminal:terminal.clone(),envelope:progress.envelope,transport_broken:false,journaled:false,
            completed:Some(CompletedRun { request:job.request.clone(),key,plan:job.plan.clone(),terminal,
                envelope:progress.envelope,native,resumed:job.resume.is_some(),journaled:false,journal_uncertain:job.journal.is_some(),terminal_owed:job.journal.is_some(),journal:job.journal.clone() }) }
    }
    /// Body text accepted so far (lossy only at a short-write cut inside a character).
    pub fn accepted_body(&self, plan: &Plan) -> Option<String> {
        let b = plan.body.as_deref()?.as_bytes();
        let n = self.envelope.body_offset.min(b.len());
        (n > 0).then(|| String::from_utf8_lossy(&b[..n]).into_owned())
    }
}

/// The planned OS calls from `from`: phase + bytes. Body calls are cut at UTF-8 boundaries.
fn calls_for(plan: &Plan, from: &Envelope) -> Vec<(Phase, Vec<u8>)> {
    let mut out = Vec::new();
    if plan.clear_first && !from.cleared {
        out.push((Phase::Clear, CLEAR.to_vec()));
    }
    if let Some(body) = &plan.body {
        if plan.fenced && !from.opened {
            out.push((Phase::OpenFence, FENCE_OPEN.to_vec()));
        }
        let b = body.as_bytes();
        let mut i = from.body_offset.min(b.len());
        while i < b.len() {
            let mut end = (i + MAX_CALL_BYTES).min(b.len());
            while end > i + 1 && end < b.len() && !body.is_char_boundary(end) {
                end -= 1;
            }
            out.push((Phase::Body, b[i..end].to_vec()));
            i = end;
        }
        if plan.fenced && !from.closed {
            out.push((Phase::CloseFence, FENCE_CLOSE.to_vec()));
        }
    }
    if let Some(c) = &plan.control {
        if !from.control_done {
            out.push((Phase::Control, c.clone()));
        }
    }
    if plan.submit.is_some() && !from.submitted {
        out.push((Phase::Submit, RETURN.to_vec()));
    }
    out
}

/// One run's accounting.
struct Run<'a> {
    request: &'a Arc<Request>,
    plan: &'a Plan,
    key: Arc<()>,
    journal: Option<Arc<dyn Journal>>,
    env: Envelope,
    acc: Accepted,
    resumed: bool,
    journaled: bool,
    journal_uncertain: bool,
}

impl Run<'_> {
    fn account(&mut self, phase: Phase, n: usize, offered: usize) {
        self.acc.calls += 1;
        let whole = n == offered;
        match phase {
            Phase::Clear => {
                self.acc.control += n as u64;
                self.env.cleared = whole;
            }
            Phase::OpenFence => {
                self.acc.fence += n as u64;
                self.env.opened = whole;
            }
            Phase::Body => {
                self.acc.payload += n as u64;
                self.env.body_offset += n;
            }
            Phase::CloseFence => {
                self.acc.fence += n as u64;
                self.env.closed = whole;
            }
            Phase::Control => {
                self.acc.control += n as u64;
                self.env.control_done = whole;
            }
            Phase::Submit => {
                self.acc.control += n as u64;
                self.env.submitted = whole;
            }
        }
    }

    /// Terminal for a stop. No byte in this run of a fresh request is a zero-write refusal or
    /// cancellation; a resumed request already owns composer bytes and is never "no write"; after
    /// bytes, only a resumable loss at a byte-exact point is PartialExact.
    fn stop(&self, reason: String, exact: bool) -> Terminal {
        if self.acc.total() == 0 && !self.resumed && self.request.native_progress().proven_zero() {
            return if reason == "cancelled" { Terminal::CancelledBeforeWrite } else { Terminal::RefusedNoWrite { reason } };
        }
        if exact {
            Terminal::PartialExact { envelope: self.env, accepted: self.acc, reason }
        } else {
            Terminal::PartialUnknown { envelope: self.env, accepted: self.acc, reason }
        }
    }

    fn done(&self, terminal: Terminal, transport_broken: bool) -> Executed {
        let Some(native)=self.request.end_run(&self.key,&terminal) else { return Executed::unowned(self.request.native_progress(),"execution_owner_conflict"); };
        Executed { terminal:terminal.clone(),envelope:self.env,transport_broken,journaled:self.journaled,
            completed:Some(CompletedRun { request:self.request.clone(),key:self.key.clone(),plan:self.plan.clone(),
                terminal,envelope:self.env,native,resumed:self.resumed,journaled:self.journaled,journal_uncertain:self.journal_uncertain,terminal_owed:self.journaled,journal:self.journal.clone() }) }
    }
}

/// Execute one job against `writer`. Never retries a whole body; never writes cleanup bytes.
pub fn execute<W: Write>(writer: &mut W, job: &AgentJob, io: &mut WriterIo<'_>) -> Executed {
    let key=match job.request.begin_run(&job.plan,job.resume,job.attempt.as_ref()) {
        Ok(key)=>key,Err(progress)=>return Executed::unowned(progress,"native_progress_conflict"),
    };
    // Catch with THIS run's private identity; a fallback reading the current request can steal
    // another run's completion after a rejected/late execution.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(||execute_started(writer,job,io,key.clone()))) {
        Ok(executed)=>executed,Err(_)=>Executed::panicked(job,key),
    }
}
fn execute_started<W:Write>(writer:&mut W,job:&AgentJob,io:&mut WriterIo<'_>,key:Arc<()>)->Executed {
    let req=&job.request;let start=job.resume.unwrap_or_default();
    let mut run=Run { request:req,plan:&job.plan,key,journal:job.journal.clone(),env:start,acc:Accepted::default(),resumed:job.resume.is_some(),journaled:false,journal_uncertain:false };
    let calls=calls_for(&job.plan,&start);
    if req.is_cancelled() {
        return run.done(run.stop("cancelled".into(), false), false);
    }
    // Journal barrier first (design 4.4 step 2); the fsync is a wait, so admission comes after it.
    if let Some(j) = &job.journal {
        let attempt = req.managed.then(|| AttemptStartReceipt {
            request:req,key:&run.key,plan:&job.plan,resume:job.resume,
        });
        run.journal_uncertain=true; // an invoked barrier can fail after installing its record
        if let Err(e) = j.writing_attempt(&ProofContext { request:req,plan:&job.plan,envelope:run.env,
            accepted:run.acc,resumed:run.resumed,phase_offset:0,next_phase:calls.first().map(|(phase,_)|*phase) },attempt.as_ref()) {
            // Hand this refusal to the settlement by claiming the guard first: a broken journal is
            // then held on backoff instead of being re-handed every tick. A lost claim stays the
            // handing-off caller's rollback.
            if job.guard.as_ref().is_some_and(|g| !g.claim_for_write()) {
                return run.done(run.stop(GUARD_ABORTED.into(), true), false);
            }
            return run.done(run.stop(format!("journal:{e}"), true), false);
        }
        run.journaled = true;
        run.journal_uncertain = false;
    }
    if job.guard.as_ref().is_some_and(|g| !g.claim_for_write()) {
        return run.done(run.stop(GUARD_ABORTED.into(), true), false);
    }
    req.set(Status::Writing);
    if let Err(loss) = job.proof.admit_context(&ProofContext { request:req, plan:&job.plan,
        envelope:run.env, accepted:run.acc, resumed:run.resumed, phase_offset:0, next_phase:calls.first().map(|(phase,_)|*phase) }) {
        req.mark_lost();
        return run.done(run.stop(loss.reason(), loss.resumable()), false);
    }
    let mut proven = true; // the admission just above proves the next call
    let mut first_byte = true;
    let mut prev: Option<Phase> = None;
    for (phase, bytes) in calls {
        // Spacing that already exists in the transport is kept; every wait voids the proof.
        if prev == Some(Phase::Clear) && job.plan.clear_settle_ms > 0 {
            (io.sleep)(Duration::from_millis(job.plan.clear_settle_ms));
            proven = false;
        }
        if phase == Phase::Submit {
            if let Some(s) = job.plan.submit {
                if s.delay_ms > 0 {
                    (io.sleep)(Duration::from_millis(s.delay_ms));
                    proven = false;
                }
                let since = io.last_program_write.map(|t| t.elapsed());
                if let Some(ms) = crate::state::cr_gap_delay_ms(since, s.min_gap_ms) {
                    (io.sleep)(Duration::from_millis(ms));
                    proven = false;
                }
            }
        }
        prev = Some(phase);
        let mut off = 0;
        // An empty chunk (control Some(vec![])) never reaches run.account, so its envelope
        // flag stays false. pending_aftercare_envelope_tests pins the validator for the flipped
        // (forged) case; re-check that specimen if this loop ever accounts an empty chunk.
        while off < bytes.len() {
            if req.is_cancelled() {
                return run.done(run.stop("cancelled".into(), false), false);
            }
            if !proven {
                if let Err(loss) = job.proof.recheck_context(phase, &ProofContext { request:req, plan:&job.plan,
                    envelope:run.env, accepted:run.acc, resumed:run.resumed, phase_offset:off, next_phase:Some(phase) }) {
                    req.mark_lost();
                    // Byte-exact resume point: inside the body (its offset is exact) or before the
                    // first byte of a call. A half-written fence/control/Return is not resumable.
                    let exact = loss.resumable() && (phase == Phase::Body || off == 0);
                    return run.done(run.stop(loss.reason(), exact), false);
                }
            }
            if first_byte {
                (io.on_first_byte)();
                first_byte = false;
            }
            proven = false;
            let slice = &bytes[off..];
            let native=req.native_call(run.env,run.acc,true);
            match writer.write(slice) {
                Ok(0) => {
                    run.acc.calls+=1; native.complete(0,run.env,run.acc);
                    return run.done(run.stop("transport:write_zero".into(), false), true);
                },
                Ok(n) => {
                    run.account(phase, n, slice.len());
                    native.complete(n,run.env,run.acc);
                    off += n;
                    if phase == Phase::Body {
                        if let Some(body) = job.plan.body.as_deref() {
                            if run.env.body_offset == body.len() {
                                (io.on_body_written)(body);
                            }
                        }
                    }
                    let flush=req.native_call(run.env,run.acc,false);
                    let result=writer.flush();
                    flush.complete(0,run.env,run.acc);
                    if let Err(e) = result {
                        return run.done(run.stop(format!("transport:{e}"), false), true);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    run.acc.calls+=1; native.complete(0,run.env,run.acc);
                },
                Err(e) => {
                    // Write::write's Err contract accepts no bytes; native panics use the RAII unknown path.
                    run.acc.calls+=1; native.complete(0,run.env,run.acc);
                    return run.done(run.stop(format!("transport:{e}"), false), true);
                },
            }
        }
    }
    let p = &job.plan;
    let return_only = p.body.is_none() && p.control.is_none();
    if (p.submit.is_none() && p.body.is_some()) || (p.submit.is_some() && return_only) {
        *io.last_program_write = Some(Instant::now());
    }
    let t = if p.submit.is_some() {
        Terminal::WrittenSubmitted { accepted: run.acc }
    } else {
        Terminal::WrittenBody { accepted: run.acc }
    };
    run.done(t, false)
}

/// Explicit refusal for a managed job that never entered its dispatch channel.
pub(crate) fn decline_unstarted(job:AgentJob,reason:&str)->Terminal {
    if !job.request.managed {
        return Terminal::PartialUnknown { envelope:job.request.native_progress().envelope,
            accepted:Accepted::default(),reason:"unmanaged_dispatch_decline".into() };
    }
    let key=match job.request.begin_run(&job.plan,job.resume,job.attempt.as_ref()) {
        Ok(key)=>key,Err(progress)=>return finish(job,&Executed::unowned(progress,"dispatch_decline_owner_conflict")),
    };
    // This path contains no native writer, proof collection, sleeps, cleanup byte or
    // mark_lost call. The actual pre-existing prefix and its original loss time remain.
    let ex={
        let run=Run { request:&job.request,plan:&job.plan,key,journal:job.journal.clone(),
            env:job.resume.unwrap_or_default(),acc:Accepted::default(),resumed:job.resume.is_some(),
            journaled:false,journal_uncertain:false };
        let cancelled=job.request.is_cancelled();
        let reason=if cancelled {"cancelled".to_owned()}else{format!("dispatch:{reason}")};
        run.done(run.stop(reason,!cancelled),false)
    };
    finish(job,&ex)
}

/// Record the terminal (journal first) and run the producer's settlement exactly once.
pub fn finish(job: AgentJob, ex: &Executed) -> Terminal {
    let Some(completed)=&ex.completed else { return Terminal::PartialUnknown { envelope:ex.envelope,
        accepted:Accepted::default(),reason:"completion_owner_conflict".into() }; };
    let lease_matches = if completed.request.managed {
        job.attempt.as_ref().is_some_and(|lease| Arc::ptr_eq(&lease.request,&completed.request)
            && Arc::ptr_eq(&lease.key,&completed.key)
            && lease.plan==completed.plan && lease.resume==job.resume)
    } else { job.attempt.is_none() };
    if !Arc::ptr_eq(&job.request,&completed.request) || job.plan!=completed.plan || job.resume.is_some()!=completed.resumed
        || !lease_matches || !job.request.claim_completion(&completed.key) {
        return Terminal::PartialUnknown { envelope:job.request.native_progress().envelope,
            accepted:Accepted::default(),reason:"completion_owner_conflict".into() };
    }
    let mut completing=Completing { request:&job.request,key:&completed.key,release_attempt:true };
    let mut terminal=completed.terminal.clone();
    // Managed Prepared needs an actual completion receipt even if cancellation or
    // barrier refusal happened before confirmed Writing. Legacy ordinary refusal
    // behavior is unchanged. A panic's journal presence is uncertainty, not proof.
    if completed.terminal_owed || completed.request.managed {
        if let Some(j) = &completed.journal {
            let receipt=WriterReceipt { completed };
            let persisted=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||j.terminal_receipt(&receipt)))
                .unwrap_or_else(|_|Err(std::io::Error::other("terminal journal panicked")));
            if let Err(e) = persisted {
                // Physical zero does not prove a durable Prepared/Writing outcome. Keep the
                // managed reservation for a typed retry/reconciliation of this receipt.
                if completed.request.managed { completing.release_attempt=false; }
                // Terminal persistence failed after possible bytes: never report success (4.5).
                if let Terminal::WrittenBody { accepted } | Terminal::WrittenSubmitted { accepted }
                    | Terminal::PartialExact { accepted,.. } | Terminal::PartialUnknown { accepted,.. } = terminal {
                    terminal = Terminal::PartialUnknown {
                        envelope: completed.envelope,
                        accepted,
                        reason: format!("journal_terminal:{e}"),
                    };
                }
            }
        }
    }
    job.request.end_run(&completed.key,&terminal);
    job.request.set(Status::Terminal(terminal.clone()));
    if let Some(settle) = job.settle {
        let t = terminal.clone();
        // A panicking settlement must not kill the writer thread.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || settle(&t)));
    }
    job.request.settled.store(true, Ordering::Release);
    terminal
}

/// What a direct-send handler can tell its caller about the writer journal (B-360).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalBarrier {
    /// The writer passed the journal barrier (`Writing` is set only after it), or the request ended for a reason
    /// that is not the barrier — those keep their existing reporting.
    Passed,
    /// The journal barrier refused the request: zero bytes. The writer's reason (`journal:<os error>`).
    Refused(String),
    /// The budget ended before the seat's writer reached the request (it is queued behind earlier writes).
    Pending,
}

/// Same budget as the schedule push's wait for its writer verdict (`schedule::PUSH_SETTLE_BUDGET_MS`).
pub const JOURNAL_BARRIER_BUDGET_MS: u64 = 2_000;

/// Wait, bounded, until the writer has passed or failed the journal barrier for `req` (B-360). The barrier is the
/// writer's first step after a cancel check (`execute_started`), so this does not wait for the plan's gaps or bytes:
/// on an idle seat it costs the journal's own fsync. Open-time and append-time journal failures look the same here
/// (`journal:` is produced only by the barrier). Call it with no lock held that the settlement takes (`input_gate`).
pub fn await_journal_barrier(req: &Request, budget: Duration) -> JournalBarrier {
    let deadline = Instant::now() + budget;
    loop {
        match req.status() {
            Status::Writing => return JournalBarrier::Passed,
            Status::Terminal(Terminal::RefusedNoWrite { reason }) if reason.starts_with("journal:") => {
                return JournalBarrier::Refused(reason)
            }
            Status::Terminal(_) => return JournalBarrier::Passed,
            Status::AcceptedPending => {}
        }
        if Instant::now() >= deadline {
            return JournalBarrier::Pending;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
thread_local! {
    static SYNC_SETTLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Tests only: specimens written against the former settle-at-claim timing see the settlement of
/// the request they just handed off (bounded 10 s wait on the calling test thread). A specimen that
/// observes the asynchronous window turns it off with [`set_test_sync_settle`]. No-op in the product.
pub fn test_wait_settled(req: &Request) {
    #[cfg(test)]
    {
        if SYNC_SETTLE.with(|c| c.get()) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !req.is_settled() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    #[cfg(not(test))]
    let _ = req;
}

#[cfg(test)]
pub fn set_test_sync_settle(on: bool) {
    SYNC_SETTLE.with(|c| c.set(on));
}

/// Proof used while a seat has no receiver binding wired yet: every check passes (the caller's
/// existing gates and hand-off guard remain the admission).
pub struct PassThrough;

impl Proof for PassThrough {
    fn admit(&self) -> Result<(), ProofLoss> {
        Ok(())
    }
    fn recheck(&self, _: Phase) -> Result<(), ProofLoss> {
        Ok(())
    }
}

/// The journal could not be opened: every barrier fails, so every write is refused before bytes.
pub struct UnavailableJournal(pub String);

impl Journal for UnavailableJournal {
    fn writing(&self, _: &Request) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0.clone()))
    }
    fn terminal(&self, _: &Request, _: &Terminal, _: &Envelope) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0.clone()))
    }
}

/// S4-1 PartialExact resume window, from proof loss (HEAD-RULINGS-W4-1 D3; locked in
/// LOCK-W4-WINDOWS-AND-N-20261016.json before any measurement).
pub const PARTIAL_EXACT_RESUME_WINDOW_MS: u64 = 30_000;

/// File journal size that triggers compaction to the still-open requests.
pub const JOURNAL_COMPACT_BYTES: u64 = 1 << 20;

/// A `writing` record left without a terminal by an earlier daemon run: restores as
/// PartialUnknown (non-replaying reconciliation, design 4.4 step 2 / 4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inconclusive {
    pub id: String,
    pub surface: u64,
    pub origin: String,
    pub digest: String,
}

/// Append-only JSONL journal, `sync_data` per record. Appenders share a read lock (no lock is held
/// across another surface's fsync); compaction alone takes the write lock. Records carry ids,
/// digests and byte counts — never body text.
pub struct FileJournal {
    path: PathBuf,
    file: RwLock<std::fs::File>,
    /// Requests whose last durable record is `writing` (leaf lock; taken under `file`).
    open: Mutex<HashMap<String, String>>,
    bytes: AtomicU64,
    compact_at: u64,
}

impl FileJournal {
    pub fn open(path: PathBuf) -> std::io::Result<(FileJournal, Vec<Inconclusive>)> {
        Self::open_with_limit(path, JOURNAL_COMPACT_BYTES)
    }

    pub fn open_with_limit(path: PathBuf, compact_at: u64) -> std::io::Result<(FileJournal, Vec<Inconclusive>)> {
        let mut open: HashMap<String, String> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        // A resumed request (S4-1) writes a second `writing` record after its partial terminal: listed once.
        let mut listed: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            // A torn last line (crash mid-append) does not parse and is ignored.
            for line in text.lines() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                let Some(id) = v["id"].as_str() else { continue };
                match v["rec"].as_str() {
                    Some("writing") => {
                        if open.insert(id.to_string(), line.to_string()).is_none() && listed.insert(id.to_string()) {
                            order.push(id.to_string());
                        }
                    }
                    Some("terminal") => {
                        open.remove(id);
                    }
                    _ => {}
                }
            }
        }
        let inconclusive = order
            .iter()
            .filter_map(|id| open.get(id))
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .map(|v| Inconclusive {
                id: v["id"].as_str().unwrap_or_default().to_string(),
                surface: v["surface"].as_u64().unwrap_or_default(),
                origin: v["origin"].as_str().unwrap_or_default().to_string(),
                digest: v["digest"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        let j = FileJournal { path, file: RwLock::new(file), open: Mutex::new(open), bytes: AtomicU64::new(bytes), compact_at };
        Ok((j, inconclusive))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append(f: &std::fs::File, line: &str) -> std::io::Result<()> {
        let mut w = f;
        w.write_all(format!("{line}\n").as_bytes())?;
        f.sync_data()
    }

    /// Rewrite the file to the still-open `writing` records (atomic rename). A failure keeps the
    /// old file: nothing is lost, the file just stays large.
    fn maybe_compact(&self) {
        if self.bytes.load(Ordering::Relaxed) < self.compact_at {
            return;
        }
        let mut file = self.file.write().unwrap_or_else(|e| e.into_inner());
        if self.bytes.load(Ordering::Relaxed) < self.compact_at {
            return;
        }
        let lines: Vec<String> = self.open.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        let tmp = self.path.with_extension("jsonl.tmp");
        let rewrite = || -> std::io::Result<std::fs::File> {
            let mut t = std::fs::File::create(&tmp)?;
            for l in &lines {
                t.write_all(format!("{l}\n").as_bytes())?;
            }
            t.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            sync_parent_dir(&self.path);
            std::fs::OpenOptions::new().append(true).open(&self.path)
        };
        if let Ok(f) = rewrite() {
            self.bytes.store(f.metadata().map(|m| m.len()).unwrap_or(0), Ordering::Relaxed);
            *file = f;
        }
    }
}

/// Make a rename durable (Unix: fsync the directory; Windows has no directory handle to sync).
fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(d) = path.parent().and_then(|dir| std::fs::File::open(dir).ok()) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

impl Journal for FileJournal {
    fn writing(&self, req: &Request) -> std::io::Result<()> {
        let line = json!({
            "rec": "writing",
            "id": req.id,
            "surface": req.surface_id,
            "origin": req.origin,
            "digest": req.digest,
            "ts": crate::state::now_epoch(),
        })
        .to_string();
        let f = self.file.read().unwrap_or_else(|e| e.into_inner());
        self.open.lock().unwrap_or_else(|e| e.into_inner()).insert(req.id.clone(), line.clone());
        let r = Self::append(&f, &line);
        if r.is_err() {
            self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&req.id);
        }
        self.bytes.fetch_add(line.len() as u64 + 1, Ordering::Relaxed);
        r
    }

    fn terminal(&self, req: &Request, t: &Terminal, envelope: &Envelope) -> std::io::Result<()> {
        let acc = match t {
            Terminal::WrittenBody { accepted }
            | Terminal::WrittenSubmitted { accepted }
            | Terminal::PartialExact { accepted, .. }
            | Terminal::PartialUnknown { accepted, .. } => *accepted,
            _ => Accepted::default(),
        };
        let reason = match t {
            Terminal::RefusedNoWrite { reason }
            | Terminal::PartialExact { reason, .. }
            | Terminal::PartialUnknown { reason, .. } => Some(reason.as_str()),
            _ => None,
        };
        let line = json!({
            "rec": "terminal",
            "id": req.id,
            "kind": t.kind(),
            "reason": reason,
            "payload": acc.payload,
            "fence": acc.fence,
            "control": acc.control,
            "calls": acc.calls,
            "envelope": {
                "cleared": envelope.cleared,
                "opened": envelope.opened,
                "body_offset": envelope.body_offset,
                "closed": envelope.closed,
                "control_done": envelope.control_done,
                "submitted": envelope.submitted,
            },
            "ts": crate::state::now_epoch(),
        })
        .to_string();
        {
            let f = self.file.read().unwrap_or_else(|e| e.into_inner());
            Self::append(&f, &line)?;
            self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&req.id);
            self.bytes.fetch_add(line.len() as u64 + 1, Ordering::Relaxed);
        }
        self.maybe_compact();
        Ok(())
    }
}

// Pending-store receipts resolve only the original native exclusion.
// These bridges cannot mint durable success or reset native progress.
/// Resolve only a committed actual zero-write outcome of THIS managed attempt.
impl Request {
    pub(crate) fn resolve_durable_zero_write(self:&Arc<Self>,
        receipt:crate::pending_requests::attempts::DurableZeroWriteOutcome)->Result<(),LeaseError> {
        let (stamp,actual)=receipt.consume(self).map_err(|_|LeaseError::ResolutionMismatch)?;
        if !self.managed || !Arc::ptr_eq(self,&stamp.request) || stamp.resume.is_some() {
            return Err(LeaseError::WrongRequest);
        }
        let mut state=self.native.lock().unwrap_or_else(|e|e.into_inner());
        if state.running || state.settling || state.completion_key.is_some()
            || !state.progress.proven_zero() || state.progress!=actual || state.plan.as_ref()!=Some(&stamp.plan) {
            return Err(LeaseError::NativeConflict);
        }
        if state.operation.is_none() && state.last_zero_resolution.as_ref().is_some_and(|key|Arc::ptr_eq(key,&stamp.key)) {
            return Ok(()); // acknowledgement only; no new authority/state transition
        }
        if !matches!(&state.operation,Some(ManagedOperation::Attempt { key,phase:AttemptPhase::Started })
            if Arc::ptr_eq(key,&stamp.key)) { return Err(LeaseError::WrongLease); }
        state.last_zero_resolution=Some(stamp.key.clone());
        state.operation=None;
        // Leave cancellation, final_outcome, status, Plan, age and native facts intact.
        // New execution still requires its own reservation and current full proof.
        Ok(())
    }
}

// No public success constructor: pending_requests alone issues these tokens from
// the real store CAS result. This bridge only discharges existing native exclusion.
impl ZeroWriteEditLease {
    pub(crate) fn resolve_selection(self,
        receipt:crate::pending_requests::DurableSelectionCommit,expected:u64)
        ->Result<u64,LeaseError>
    {
        let plan=self.original_plan.as_ref().ok_or(LeaseError::PlanChanged)?;
        let (stamp,revision)=receipt.consume(&self.request,plan,expected)
            .map_err(|_|LeaseError::ResolutionMismatch)?;
        self.resolve(DurableEditResolution::Committed(DurableEditCommit { stamp }))?;
        Ok(revision)
    }
    pub(crate) fn resolve_selection_not_written(self,
        receipt:crate::pending_requests::DurableSelectionNotWritten,expected:u64)
        ->Result<(),LeaseError>
    {
        let plan=self.original_plan.as_ref().ok_or(LeaseError::PlanChanged)?;
        let stamp=receipt.consume(&self.request,plan,expected)
            .map_err(|_|LeaseError::ResolutionMismatch)?;
        self.resolve(DurableEditResolution::NotWritten(DurableEditNoChange { stamp }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const KO: &str = "가나다라마바사아자차카타파하";

    fn long_body() -> String {
        // > 2 x 4,096 bytes of 3-byte characters with an ASCII run, so cuts land inside characters.
        let mut s = String::new();
        while s.len() < 10_000 {
            s.push_str(KO);
            s.push_str(" line\n");
        }
        s
    }

    /// Scripted receiver proof: `admit`, then the n-th `recheck` (0-based) fails with `fail`.
    struct FakeProof {
        admit: Result<(), ProofLoss>,
        fail: Mutex<Option<(usize, ProofLoss)>>,
        rechecks: Mutex<Vec<Phase>>,
        log: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl FakeProof {
        fn healthy() -> Arc<FakeProof> {
            Arc::new(FakeProof { admit: Ok(()), fail: Mutex::new(None), rechecks: Mutex::new(Vec::new()), log: None })
        }
        fn failing_at(n: usize, loss: ProofLoss) -> Arc<FakeProof> {
            Arc::new(FakeProof { admit: Ok(()), fail: Mutex::new(Some((n, loss))), rechecks: Mutex::new(Vec::new()), log: None })
        }
        fn refusing(loss: ProofLoss) -> Arc<FakeProof> {
            Arc::new(FakeProof { admit: Err(loss), fail: Mutex::new(None), rechecks: Mutex::new(Vec::new()), log: None })
        }
        fn rechecks(&self) -> usize {
            self.rechecks.lock().unwrap().len()
        }
    }

    impl Proof for FakeProof {
        fn admit(&self) -> Result<(), ProofLoss> {
            if let Some(l) = &self.log {
                l.lock().unwrap().push("admit".into());
            }
            self.admit.clone()
        }
        fn recheck(&self, phase: Phase) -> Result<(), ProofLoss> {
            let mut r = self.rechecks.lock().unwrap();
            let n = r.len();
            r.push(phase);
            match &*self.fail.lock().unwrap() {
                Some((at, loss)) if *at == n => Err(loss.clone()),
                _ => Ok(()),
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Step {
        Short(usize),
        Intr,
        Fail,
        /// Accept the call fully, then cancel the request (a cancellation that arrives while a call is issued).
        FullThenCancel,
    }

    /// Records each accepted call; follows a script of short writes / EINTR / errors.
    struct Rec {
        calls: Vec<Vec<u8>>,
        attempts: usize,
        script: VecDeque<Step>,
        cancel: Option<Arc<Request>>,
        log: Option<Arc<Mutex<Vec<String>>>>,
    }

    impl Rec {
        fn new() -> Rec {
            Rec { calls: Vec::new(), attempts: 0, script: VecDeque::new(), cancel: None, log: None }
        }
        fn bytes(&self) -> Vec<u8> {
            self.calls.concat()
        }
    }

    impl Write for Rec {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.attempts += 1;
            if let Some(l) = &self.log {
                l.lock().unwrap().push("write".into());
            }
            let n = match self.script.pop_front() {
                Some(Step::Short(n)) => n.min(buf.len()),
                Some(Step::Intr) => return Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
                Some(Step::Fail) => return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
                Some(Step::FullThenCancel) => {
                    if let Some(r) = &self.cancel {
                        r.cancel();
                    }
                    buf.len()
                }
                None => buf.len(),
            };
            self.calls.push(buf[..n].to_vec());
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn job(plan: Plan, proof: Arc<FakeProof>) -> AgentJob {
        AgentJob {
            request: Request::new("w-test".into(), 7, "test", "d".into()),
            plan,
            proof,
            journal: None,
            attempt: None,
            resume: None,
            settle: None,
            guard: None,
            note_body: true,
            body_owner: None,
        }
    }

    fn run_job(w: &mut Rec, j: &AgentJob) -> Executed {
        let mut anchor = None;
        run_job_with(w, j, &mut anchor, &|_| {})
    }

    fn run_job_with(w: &mut Rec, j: &AgentJob, anchor: &mut Option<Instant>, sleep: &dyn Fn(Duration)) -> Executed {
        let mut first = || {};
        let mut body_done = |_: &str| {};
        let mut io = WriterIo { sleep, last_program_write: anchor, on_first_byte: &mut first, on_body_written: &mut body_done };
        execute(w, j, &mut io)
    }

    fn count(hay: &[u8], needle: &[u8]) -> usize {
        hay.windows(needle.len()).filter(|w| *w == needle).count()
    }

    #[test]
    fn paste_plan_bytes_equal_the_existing_inject_sequence() {
        let t = format!("{}\x1b[201~tail {}", long_body(), "\u{9b}201~");
        let mut want = b"\x15".to_vec();
        want.extend_from_slice(cys::paste_fence::wrap(&t).as_bytes());
        want.push(b'\r');
        assert_eq!(Plan::paste(&t).with_clear(150).with_submit(400, 0).planned_bytes(), want);
        assert_eq!(Plan::raw("x\r\ny").planned_bytes(), b"x\r\ny");
        assert_eq!(Plan::submit_only(150).planned_bytes(), b"\r");
    }

    /// S-T03 healthy arm: a long UTF-8 body > 4,096 bytes arrives once, inside one envelope, with
    /// one Return; every call is capped and every call after the admitted first one is re-proven.
    #[test]
    fn long_utf8_body_is_capped_cut_on_char_boundaries_and_rechecked_before_every_call() {
        let body = long_body();
        let plan = Plan::paste(&body).with_clear(0).with_submit(0, 0);
        let p = FakeProof::healthy();
        let j = job(plan.clone(), p.clone());
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        let out = w.bytes();
        assert_eq!(out, plan.planned_bytes());
        assert!(w.calls.len() >= 6, "clear + open + >=3 body + close + return");
        assert!(w.calls.iter().all(|c| c.len() <= MAX_CALL_BYTES));
        for c in &w.calls[2..w.calls.len() - 2] {
            assert!(std::str::from_utf8(c).is_ok(), "body call cut inside a character");
        }
        assert_eq!((count(&out, FENCE_OPEN), count(&out, FENCE_CLOSE), count(&out, b"\r")), (1, 1, 1));
        assert_eq!(p.rechecks(), w.calls.len() - 1, "every call after the admitted first one is re-proven");
        match ex.terminal {
            Terminal::WrittenSubmitted { accepted } => {
                assert_eq!(accepted.payload, body.len() as u64);
                assert_eq!(accepted.fence, (FENCE_OPEN.len() + FENCE_CLOSE.len()) as u64);
                assert_eq!(accepted.control, 2);
            }
            t => panic!("{t:?}"),
        }
    }

    /// S-T03 chunk exit: death proven before call k+1 (after open fence, after a payload prefix,
    /// after the close fence, before the Return, after Ctrl-U) writes nothing after call k.
    #[test]
    fn death_at_every_call_boundary_writes_nothing_after_it() {
        let body = long_body();
        for clear in [false, true] {
            let mut plan = Plan::paste(&body).with_submit(0, 0);
            if clear {
                plan = plan.with_clear(0);
            }
            let calls = calls_for(&plan, &Envelope::default());
            for k in 0..calls.len() - 1 {
                let p = FakeProof::failing_at(k, ProofLoss::AgentDead);
                let j = job(plan.clone(), p);
                let mut w = Rec::new();
                let ex = run_job(&mut w, &j);
                let want: Vec<u8> = calls[..=k].iter().flat_map(|(_, b)| b.clone()).collect();
                assert_eq!(w.bytes(), want, "clear={clear} k={k}: bytes after the failed proof");
                assert_eq!(count(&w.bytes(), b"\r"), 0, "clear={clear} k={k}: a Return reached a dead receiver");
                match ex.terminal {
                    Terminal::PartialUnknown { accepted, reason, .. } => {
                        assert_eq!(accepted.total(), want.len() as u64);
                        assert_eq!(reason, "agent_dead");
                    }
                    t => panic!("clear={clear} k={k}: {t:?}"),
                }
            }
        }
    }

    /// S4-1 at the writer: a non-death Unknown mid-body stops byte-exactly; the SAME request resumes
    /// from its envelope and the two runs together are the planned bytes exactly once.
    #[test]
    fn resumable_loss_mid_body_is_partial_exact_and_resume_completes_once() {
        let body = long_body();
        let plan = Plan::paste(&body).with_submit(0, 0);
        // Calls: open(0) body(1) body(2) body(3) close(4) return(5); recheck index 1 precedes call 2.
        let j = job(plan.clone(), FakeProof::failing_at(1, ProofLoss::Unknown("agent:denied".into())));
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        let env = match ex.terminal {
            Terminal::PartialExact { envelope, reason, .. } => {
                assert_eq!(reason, "unknown:agent:denied");
                envelope
            }
            t => panic!("{t:?}"),
        };
        assert!(env.opened && !env.closed && !env.submitted);
        assert_eq!(env.body_offset, w.bytes().len() - FENCE_OPEN.len());
        let mut resumed = job(plan.clone(), FakeProof::healthy());
        resumed.request = j.request.clone();
        resumed.resume = Some(env);
        let mut w2 = Rec::new();
        let ex2 = run_job(&mut w2, &resumed);
        assert!(matches!(ex2.terminal, Terminal::WrittenSubmitted { .. }), "{:?}", ex2.terminal);
        let mut all = w.bytes();
        all.extend(w2.bytes());
        assert_eq!(all, plan.planned_bytes());
        assert_eq!((count(&all, FENCE_OPEN), count(&all, FENCE_CLOSE), count(&all, b"\r")), (1, 1, 1));
    }

    /// Replacement or epoch movement is not resumable (S4-1: "A proven-dead or replaced binding
    /// stays PartialUnknown"), and neither is a cut inside a fence.
    #[test]
    fn replacement_epoch_move_and_half_fence_are_partial_unknown() {
        let plan = Plan::paste(&long_body()).with_submit(0, 0);
        for loss in [ProofLoss::Changed, ProofLoss::EpochMoved, ProofLoss::RootDead] {
            let j = job(plan.clone(), FakeProof::failing_at(1, loss.clone()));
            let mut w = Rec::new();
            let t = run_job(&mut w, &j).terminal;
            assert!(matches!(t, Terminal::PartialUnknown { .. }), "{loss:?} -> {t:?}");
        }
        // Open fence accepted only 3 of 6 bytes, then a resumable loss: the cut is not byte-exact.
        let j = job(plan, FakeProof::failing_at(0, ProofLoss::Unknown("x".into())));
        let mut w = Rec::new();
        w.script.push_back(Step::Short(3));
        let t = run_job(&mut w, &j).terminal;
        assert!(matches!(t, Terminal::PartialUnknown { .. }), "{t:?}");
        assert_eq!(w.bytes(), b"\x1b[2");
    }

    /// S-T07: short writes and EINTR retries are each a separate OS call with a fresh proof; the
    /// body is reconstructed exactly once.
    #[test]
    fn short_writes_and_eintr_retries_each_get_a_fresh_proof() {
        let body = long_body();
        let plan = Plan::paste(&body).with_submit(0, 0);
        let p = FakeProof::healthy();
        let j = job(plan.clone(), p.clone());
        let mut w = Rec::new();
        w.script.extend([Step::Short(2), Step::Intr, Step::Short(1), Step::Short(1000), Step::Intr, Step::Short(1)]);
        let ex = run_job(&mut w, &j);
        assert_eq!(w.bytes(), plan.planned_bytes());
        assert_eq!(p.rechecks(), w.attempts - 1, "a retry was hidden behind an earlier proof");
        match ex.terminal {
            Terminal::WrittenSubmitted { accepted } => assert_eq!(accepted.calls as usize, w.attempts),
            t => panic!("{t:?}"),
        }
        // A resumable loss right after a short body write stops at the exact body offset.
        let p = FakeProof::failing_at(1, ProofLoss::Unknown("u".into()));
        let j = job(plan.clone(), p);
        let mut w = Rec::new();
        w.script.extend([Step::Short(6), Step::Short(777)]);
        match run_job(&mut w, &j).terminal {
            Terminal::PartialExact { envelope, .. } => assert_eq!(envelope.body_offset, 777),
            t => panic!("{t:?}"),
        }
    }

    /// S-T21a: cancellation before bytes, between returned calls and during prescribed spacing;
    /// never a destructor/cleanup byte and never a later phase.
    #[test]
    fn cancellation_before_between_and_during_spacing_stops_later_calls_only() {
        let body = long_body();
        let plan = Plan::paste(&body).with_clear(5).with_submit(5, 0);
        // (a) before any byte
        let j = job(plan.clone(), FakeProof::healthy());
        j.request.cancel();
        let mut w = Rec::new();
        assert_eq!(run_job(&mut w, &j).terminal, Terminal::CancelledBeforeWrite);
        assert_eq!(w.attempts, 0);
        // (b) cancellation arrives while call 1 (open fence) is issued: it completes, nothing follows.
        let j = job(plan.clone(), FakeProof::healthy());
        let mut w = Rec::new();
        w.cancel = Some(j.request.clone());
        w.script.extend([Step::Short(1), Step::FullThenCancel]);
        let t = run_job(&mut w, &j).terminal;
        assert_eq!(w.bytes(), [CLEAR, FENCE_OPEN].concat());
        assert!(matches!(t, Terminal::PartialUnknown { ref reason, .. } if reason == "cancelled"), "{t:?}");
        // (c) cancellation during the clear settle: nothing after Ctrl-U (the submit spacing has its own test).
        let j = job(plan.clone(), FakeProof::healthy());
        let req = j.request.clone();
        let mut w = Rec::new();
        let mut anchor = None;
        let sleep = move |d: Duration| {
            if d == Duration::from_millis(5) && req.status() == Status::Writing {
                req.cancel();
            }
        };
        let t = run_job_with(&mut w, &j, &mut anchor, &sleep).terminal;
        // The first 5 ms wait is the clear settle: cancelled there, only Ctrl-U was written.
        assert_eq!(w.bytes(), CLEAR);
        assert!(matches!(t, Terminal::PartialUnknown { .. }), "{t:?}");
        let out = w.bytes();
        assert!(!out.contains(&b'\r') && !out.contains(&b'\n') && !out.contains(&0x04));
    }

    #[test]
    fn cancellation_during_submit_spacing_writes_no_return() {
        let plan = Plan::paste("short").with_submit(5, 0);
        let j = job(plan, FakeProof::healthy());
        let req = j.request.clone();
        let mut w = Rec::new();
        let mut anchor = None;
        let t = run_job_with(&mut w, &j, &mut anchor, &move |_| req.cancel()).terminal;
        assert_eq!(w.bytes(), cys::paste_fence::wrap("short").as_bytes());
        assert!(matches!(t, Terminal::PartialUnknown { envelope, .. } if envelope.closed && !envelope.submitted));
    }

    /// ★(0.14.52 · S-a C7b) A resumed request writes a second `writing` record after its partial terminal; a crash during
    /// the resumed run lists it ONCE as inconclusive (and a resumed request that completed is not listed).
    #[test]
    fn resumed_request_is_listed_once_by_the_journal() {
        let dir = std::env::temp_dir().join(format!("cys-aw-journal-resume-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("j.jsonl");
        let (j, _) = FileJournal::open(path.clone()).unwrap();
        let open_one = Request::new("r-open".into(), 7, "queue", "d1".into());
        let done = Request::new("r-done".into(), 7, "queue", "d2".into());
        let partial = Terminal::PartialExact { envelope: Envelope::default(), accepted: Accepted::default(), reason: "unknown:x".into() };
        for r in [&open_one, &done] {
            j.writing(r).unwrap();
            j.terminal(r, &partial, &Envelope::default()).unwrap();
            j.writing(r).unwrap(); // the resumed run
        }
        let submitted = Terminal::WrittenSubmitted { accepted: Accepted::default() };
        j.terminal(&done, &submitted, &Envelope::default()).unwrap();
        drop(j);
        let (_, inconclusive) = FileJournal::open(path).unwrap();
        let ids: Vec<&str> = inconclusive.iter().map(|i| i.id.as_str()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(ids, vec!["r-open"]);
    }

    /// A resumed request already owns composer bytes: a failed admission is never RefusedNoWrite.
    #[test]
    fn resumed_request_whose_admission_fails_is_never_refused_no_write() {
        // Construct the prefix through the writer, so the Request owns actual PartialExact authority.
        let held = || {
            let mut j=job(Plan::paste(&long_body()).with_submit(0,0),FakeProof::failing_at(1,ProofLoss::Unknown("initial".into())));
            let mut w=Rec::new();w.script=VecDeque::from([Step::Short(usize::MAX),Step::Short(10)]);
            let ex=run_job(&mut w,&j);assert!(matches!(ex.terminal,Terminal::PartialExact{..}));
            assert_eq!(ex.envelope,Envelope { opened:true, body_offset:10, ..Envelope::default() });
            j.resume=Some(ex.envelope);j
        };
        let cases = [
            (ProofLoss::Unknown("u".into()), "partial_exact"),
            (ProofLoss::ContraryForeground, "partial_exact"),
            (ProofLoss::AgentDead, "partial_unknown"),
            (ProofLoss::Changed, "partial_unknown"),
        ];
        for (loss, kind) in cases {
            let mut j=held();let env=j.resume.unwrap();j.proof=FakeProof::refusing(loss.clone());
            let mut w = Rec::new();
            let ex = run_job(&mut w, &j);
            assert_eq!(ex.terminal.kind(), kind, "{loss:?}");
            assert_eq!(ex.envelope, env);
            assert_eq!(w.attempts, 0);
        }
        let j=held();
        j.request.cancel();
        assert_eq!(run_job(&mut Rec::new(), &j).terminal.kind(), "partial_unknown");
        // A fresh request refused at admission writes zero bytes.
        let j = job(Plan::paste("x").with_submit(0, 0), FakeProof::refusing(ProofLoss::AgentDead));
        let mut w = Rec::new();
        assert_eq!(run_job(&mut w, &j).terminal, Terminal::RefusedNoWrite { reason: "agent_dead".into() });
        assert_eq!(w.attempts, 0);
    }

    struct LogJournal {
        log: Arc<Mutex<Vec<String>>>,
        fail_writing: bool,
        fail_terminal: bool,
    }

    impl Journal for LogJournal {
        fn writing(&self, _: &Request) -> std::io::Result<()> {
            self.log.lock().unwrap().push("writing".into());
            if self.fail_writing {
                return Err(std::io::Error::other("disk"));
            }
            Ok(())
        }
        fn terminal(&self, _: &Request, t: &Terminal, _: &Envelope) -> std::io::Result<()> {
            self.log.lock().unwrap().push(format!("terminal:{}", t.kind()));
            if self.fail_terminal {
                return Err(std::io::Error::other("disk"));
            }
            Ok(())
        }
    }

    fn journaled_job(log: &Arc<Mutex<Vec<String>>>, fail_writing: bool, fail_terminal: bool) -> AgentJob {
        let proof = Arc::new(FakeProof {
            admit: Ok(()),
            fail: Mutex::new(None),
            rechecks: Mutex::new(Vec::new()),
            log: Some(log.clone()),
        });
        let mut j = job(Plan::paste("hello").with_submit(0, 0), proof);
        j.journal = Some(Arc::new(LogJournal { log: log.clone(), fail_writing, fail_terminal }));
        let l = log.clone();
        j.settle = Some(Box::new(move |t: &Terminal| l.lock().unwrap().push(format!("settle:{}", t.kind()))));
        j
    }

    /// B-360: the handler's bounded wait sees an APPEND-time journal failure (an opened journal whose `writing`
    /// fails — the disk-full / read-only route), not only an open failure; it waits for a writer that reaches the job
    /// later; a non-journal zero-byte refusal and a written request are `Passed`; an unreached job is `Pending`.
    #[test]
    fn b360_await_journal_barrier_reports_the_barrier_verdict() {
        let budget = Duration::from_millis(JOURNAL_BARRIER_BUDGET_MS);
        // append-time refusal, reached by the writer 30 ms after the wait starts
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, true, false);
        let req = j.request.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let mut w = Rec::new();
            let ex = run_job(&mut w, &j);
            finish(j, &ex);
            w.attempts
        });
        assert_eq!(await_journal_barrier(&req, budget), JournalBarrier::Refused("journal:disk".into()));
        assert_eq!(t.join().unwrap(), 0, "zero bytes");
        // healthy journal: written
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, false, false);
        let req = j.request.clone();
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        finish(j, &ex);
        assert_eq!(await_journal_barrier(&req, budget), JournalBarrier::Passed);
        // a receiver-proof refusal is not this mechanism's question: Passed (its existing reporting stands)
        let j = job(Plan::paste("x").with_submit(0, 0), FakeProof::refusing(ProofLoss::AgentDead));
        let req = j.request.clone();
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        finish(j, &ex);
        assert!(matches!(req.status(), Status::Terminal(Terminal::RefusedNoWrite { .. })));
        assert_eq!(await_journal_barrier(&req, budget), JournalBarrier::Passed);
        // never reached by a writer: Pending once the budget ends
        let j = journaled_job(&Arc::new(Mutex::new(Vec::new())), true, false);
        let t0 = Instant::now();
        assert_eq!(await_journal_barrier(&j.request, Duration::from_millis(40)), JournalBarrier::Pending);
        assert!(t0.elapsed() >= Duration::from_millis(40));
    }

    /// Design 4.4 step 2 / 4.5: the durable `writing` record precedes admission and the first byte;
    /// the terminal record precedes settlement; settlement runs exactly once.
    #[test]
    fn journal_barrier_precedes_first_byte_and_terminal_precedes_settle() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, false, false);
        let mut w = Rec::new();
        w.log = Some(log.clone());
        let ex = run_job(&mut w, &j);
        let t = finish(j, &ex);
        assert_eq!(t.kind(), "written_submitted");
        let got = log.lock().unwrap().clone();
        assert_eq!(got, ["writing", "admit", "write", "write", "write", "write", "terminal:written_submitted", "settle:written_submitted"]);
        // A failed barrier refuses with zero bytes and never reaches admission.
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, true, false);
        let mut w = Rec::new();
        let ex = run_job(&mut w, &j);
        assert!(matches!(ex.terminal, Terminal::RefusedNoWrite { ref reason } if reason.starts_with("journal:")));
        assert_eq!(w.attempts, 0);
        finish(j, &ex);
        assert_eq!(log.lock().unwrap().clone(), ["writing", "settle:refused_no_write"]);
        // A failed terminal record after bytes never reports success.
        let log = Arc::new(Mutex::new(Vec::new()));
        let j = journaled_job(&log, false, true);
        let ex = run_job(&mut Rec::new(), &j);
        let req = j.request.clone();
        let t = finish(j, &ex);
        assert!(matches!(t, Terminal::PartialUnknown { ref reason, .. } if reason.starts_with("journal_terminal:")), "{t:?}");
        assert_eq!(req.status(), Status::Terminal(t));
        assert_eq!(log.lock().unwrap().last().unwrap(), "settle:partial_unknown");
    }

    #[test]
    fn panicking_settlement_is_contained() {
        let mut j = job(Plan::paste("x"), FakeProof::healthy());
        j.settle = Some(Box::new(|_| panic!("settle bug")));
        let ex = run_job(&mut Rec::new(), &j);
        assert_eq!(finish(j, &ex).kind(), "written_body");
    }

    /// The writer's B2' anchor keeps the existing variant semantics.
    #[test]
    fn program_write_anchor_follows_program_inject_and_submit_after_gap() {
        let none = |plan: Plan| {
            let mut a = None;
            run_job_with(&mut Rec::new(), &job(plan, FakeProof::healthy()), &mut a, &|_| {});
            a
        };
        assert!(none(Plan::raw("body")).is_some(), "body without own Return stamps (Program)");
        assert!(none(Plan::paste("body").with_submit(0, 0)).is_none(), "body + own Return does not stamp (Inject)");
        assert!(none(Plan::control(vec![0x1b])).is_none(), "control does not stamp (Data)");
        // Return-only waits out the remaining minimum gap, then stamps (SubmitAfterGap).
        let slept = Arc::new(Mutex::new(Vec::new()));
        let s2 = slept.clone();
        let mut a = Some(Instant::now() - Duration::from_millis(50));
        let p = FakeProof::healthy();
        let ex = run_job_with(&mut Rec::new(), &job(Plan::submit_only(150), p.clone()), &mut a, &move |d| s2.lock().unwrap().push(d));
        assert_eq!(ex.terminal.kind(), "written_submitted");
        let d = slept.lock().unwrap()[0];
        assert!(d >= Duration::from_millis(80) && d <= Duration::from_millis(100), "{d:?}");
        assert_eq!(p.rechecks(), 1, "the Return after the gap wait is re-proven");
        assert!(a.unwrap().elapsed() < Duration::from_millis(40));
    }

    /// Sink shared with the real writer loop; `fail` makes every write a broken pipe.
    #[derive(Clone)]
    struct Shared {
        buf: Arc<Mutex<Vec<u8>>>,
        fail: bool,
    }

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.fail {
                return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
            }
            self.buf.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Loop {
        tx: std::sync::mpsc::SyncSender<crate::state::WriteReq>,
        track: Arc<crate::state::InjectTrack>,
        stop: Arc<AtomicBool>,
        handle: std::thread::JoinHandle<()>,
        buf: Arc<Mutex<Vec<u8>>>,
    }

    /// The production writer loop (`run_writer_loop_tracked`) on a shared sink.
    fn spawn_loop(fail: bool) -> Loop {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = Shared { buf: buf.clone(), fail };
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let track = Arc::new(crate::state::InjectTrack::default());
        let (s2, t2) = (stop.clone(), track.clone());
        let handle = std::thread::spawn(move || crate::state::run_writer_loop_tracked(sink, rx, s2, Some(t2)));
        Loop { tx, track, stop, handle, buf }
    }

    impl Loop {
        fn wait_for(&self, want: &[u8]) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while *self.buf.lock().unwrap() != want {
                assert!(Instant::now() < deadline, "writer output {:?} != {:?}", self.buf.lock().unwrap(), want);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fn close(self) {
            self.stop.store(true, Ordering::Relaxed);
            drop(self.tx);
            self.handle.join().unwrap();
        }
    }

    fn settled(mut j: AgentJob) -> (AgentJob, std::sync::mpsc::Receiver<Terminal>) {
        let (tx, rx) = std::sync::mpsc::channel();
        j.settle = Some(Box::new(move |t: &Terminal| {
            let _ = tx.send(t.clone());
        }));
        (j, rx)
    }

    /// Design 4.7 / S-T21a: a safety refusal is a typed outcome — zero bytes, no track marks, and
    /// the writer loop keeps serving the next request (no exit, so no Drop newline+VEOF).
    #[test]
    fn refused_agent_job_on_the_real_writer_loop_writes_nothing_and_keeps_the_loop() {
        let l = spawn_loop(false);
        let (j, rx) = settled(job(Plan::paste("hello").with_submit(0, 0), FakeProof::refusing(ProofLoss::AgentDead)));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        l.tx.send(crate::state::WriteReq::Data(b"after".to_vec())).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Terminal::RefusedNoWrite { reason: "agent_dead".into() });
        l.wait_for(b"after");
        assert!(!l.track.busy_within(Duration::from_secs(60)), "track marked for a request that wrote nothing");
        assert!(l.track.last_body().is_none());
        l.close();
    }

    #[test]
    fn partial_agent_job_on_the_real_writer_loop_keeps_the_loop_and_notes_only_the_prefix() {
        let l = spawn_loop(false);
        let body = long_body();
        let plan = Plan::paste(&body).with_submit(0, 0);
        let (j, rx) = settled(job(plan.clone(), FakeProof::failing_at(1, ProofLoss::AgentDead)));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        l.tx.send(crate::state::WriteReq::Data(b"after".to_vec())).unwrap();
        let t = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let Terminal::PartialUnknown { envelope, .. } = t else { panic!("{t:?}") };
        let prefix = &body.as_bytes()[..envelope.body_offset];
        let mut want = FENCE_OPEN.to_vec();
        want.extend_from_slice(prefix);
        want.extend_from_slice(b"after");
        l.wait_for(&want);
        let noted = l.track.last_body().expect("accepted prefix noted").norm;
        assert_eq!(noted, crate::state::normalize_input_text(std::str::from_utf8(prefix).unwrap()));
        assert_eq!(l.track.submit_settle_obs(crate::state::settle_mono_ms(), 60_000).since_written_ms, None);
        l.close();
    }

    #[test]
    fn healthy_agent_job_on_the_real_writer_loop_submits_once_and_marks_the_track() {
        let l = spawn_loop(false);
        let plan = Plan::paste("hello agent").with_clear(1).with_submit(1, 0);
        let (j, rx) = settled(job(plan.clone(), FakeProof::healthy()));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().kind(), "written_submitted");
        l.wait_for(&plan.planned_bytes());
        assert_eq!(l.track.last_body().unwrap().norm, "hello agent");
        assert!(l.track.submit_settle_obs(crate::state::settle_mono_ms(), 60_000).since_written_ms.is_some());
        assert!(l.track.busy_within(Duration::from_secs(60)));
        l.close();
    }

    /// A genuinely broken transport still ends the loop, as before; the record says so truthfully.
    #[test]
    fn transport_error_still_ends_the_writer_loop_with_a_truthful_record() {
        let l = spawn_loop(true);
        let (j, rx) = settled(job(Plan::paste("hello").with_submit(0, 0), FakeProof::healthy()));
        l.tx.send(crate::state::WriteReq::Agent(Box::new(j))).unwrap();
        let t = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(t, Terminal::RefusedNoWrite { ref reason } if reason.starts_with("transport:")), "{t:?}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !l.handle.is_finished() {
            assert!(Instant::now() < deadline, "writer loop survived a broken transport");
            std::thread::sleep(Duration::from_millis(5));
        }
        l.close();
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cys-agent-write-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn file_journal_restores_writing_without_terminal_as_inconclusive_and_compacts() {
        let dir = scratch("journal");
        let path = dir.join("writer-journal.jsonl");
        let r1 = Request::new("w1".into(), 3, "queue", "aa".into());
        let r2 = Request::new("w2".into(), 4, "send", "bb".into());
        {
            let (j, inc) = FileJournal::open_with_limit(path.clone(), 2_000).unwrap();
            assert!(inc.is_empty());
            j.writing(&r1).unwrap();
            j.terminal(&r1, &Terminal::WrittenSubmitted { accepted: Accepted::default() }, &Envelope::default()).unwrap();
            j.writing(&r2).unwrap(); // the daemon "crashes" here
            for i in 0..40 {
                let r = Request::new(format!("x{i}"), 5, "queue", "cc".into());
                j.writing(&r).unwrap();
                j.terminal(&r, &Terminal::CancelledBeforeWrite, &Envelope::default()).unwrap();
            }
            assert!(std::fs::metadata(&path).unwrap().len() < 2_000, "compaction did not run");
        }
        // A torn final line from a crash mid-append is ignored.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"rec\":\"term").unwrap();
        drop(f);
        let (_, inc) = FileJournal::open(path.clone()).unwrap();
        assert_eq!(inc, vec![Inconclusive { id: "w2".into(), surface: 4, origin: "send".into(), digest: "bb".into() }]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("hello") && !text.contains("가나"), "journal must not carry body text");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ContextRow { admit: bool, phase: Option<Phase>, offset: usize, body: usize, control: u64, payload: u64, resumed: bool }
    struct ContextProof { rows: Mutex<Vec<ContextRow>>, request: Arc<Request>, plan: Plan }
    impl ContextProof {
        fn record(&self, phase: Option<Phase>, ctx: &ProofContext<'_>) {
            assert!(std::ptr::eq(ctx.request, self.request.as_ref()), "proof context must retain the original request object");
            assert!(!ctx.request.is_settled(), "an active resumed run cannot remain marked settled from its prior run");
            assert_eq!(ctx.plan, &self.plan, "proof context must retain every immutable plan field");
            self.rows.lock().unwrap().push(ContextRow { admit: phase.is_none(), phase, offset: ctx.phase_offset,
                body: ctx.envelope.body_offset, control: ctx.accepted.control, payload: ctx.accepted.payload, resumed: ctx.resumed });
        }
    }
    impl Proof for ContextProof {
        fn admit(&self) -> Result<(), ProofLoss> { Ok(()) }
        fn recheck(&self, _: Phase) -> Result<(), ProofLoss> { Ok(()) }
        fn admit_context(&self, ctx: &ProofContext<'_>) -> Result<(), ProofLoss> { self.record(None,ctx); Ok(()) }
        fn recheck_context(&self, phase: Phase, ctx: &ProofContext<'_>) -> Result<(), ProofLoss> { self.record(Some(phase),ctx); Ok(()) }
    }

    #[test]
    fn c8i_context_reaches_admission_short_retry_interrupted_retry_and_submit() {
        let mut j=job(Plan::raw("abcdef").with_clear(1).with_submit(1,0),FakeProof::healthy());
        let p=Arc::new(ContextProof{rows:Mutex::new(Vec::new()),request:j.request.clone(),plan:j.plan.clone()});j.proof=p.clone();
        let mut w=Rec::new();w.script=VecDeque::from([Step::Short(1),Step::Short(2),Step::Intr,Step::Short(4),Step::Short(1)]);
        let ex=run_job(&mut w,&j);assert!(matches!(ex.terminal,Terminal::WrittenSubmitted{..}));
        let got=p.rows.lock().unwrap().clone();
        let want=vec![
            ContextRow{admit:true,phase:None,offset:0,body:0,control:0,payload:0,resumed:false},
            ContextRow{admit:false,phase:Some(Phase::Body),offset:0,body:0,control:1,payload:0,resumed:false},
            ContextRow{admit:false,phase:Some(Phase::Body),offset:2,body:2,control:1,payload:2,resumed:false},
            ContextRow{admit:false,phase:Some(Phase::Body),offset:2,body:2,control:1,payload:2,resumed:false},
            ContextRow{admit:false,phase:Some(Phase::Submit),offset:0,body:6,control:1,payload:6,resumed:false},
        ];
        assert_eq!(got,want,"each proof sees the exact writer-owned progress at its call boundary");
    }

    #[test]
    fn c8i_resumed_admission_keeps_original_prefix_and_per_run_counts() {
        let mut j=job(Plan::raw("abcdef").with_submit(0,0),FakeProof::healthy());
        j.proof=FakeProof::failing_at(0,ProofLoss::Unknown("once".into()));
        let mut first=Rec::new();first.script=VecDeque::from([Step::Short(2)]);
        let first_ex=run_job(&mut first,&j);assert!(matches!(first_ex.terminal,Terminal::PartialExact{..}));
        assert_eq!(first.bytes(),b"ab");
        let request=j.request.clone();let plan=j.plan.clone();finish(j,&first_ex);
        assert!(request.is_settled(),"premise: first run settled");
        let mut j=job(plan,FakeProof::healthy());j.request=request;j.resume=Some(first_ex.envelope);
        let p=Arc::new(ContextProof{rows:Mutex::new(Vec::new()),request:j.request.clone(),plan:j.plan.clone()});j.proof=p.clone();
        let mut w=Rec::new();let ex=run_job(&mut w,&j);assert!(matches!(ex.terminal,Terminal::WrittenSubmitted{..}));
        assert_eq!(w.bytes(),b"cdef\r");
        assert_eq!(*p.rows.lock().unwrap(),vec![
            ContextRow{admit:true,phase:None,offset:0,body:2,control:0,payload:0,resumed:true},
            ContextRow{admit:false,phase:Some(Phase::Submit),offset:0,body:6,control:0,payload:4,resumed:true},
        ],"resume proof sees cumulative prefix and only this run's accepted bytes");
        let progress=j.request.native_progress();
        assert_eq!(progress.accepted_total,7,"same-request resume preserves both runs' accepted bytes");
        assert_eq!(progress.run_accepted.payload,4,"per-run accounting resets independently");
        assert_eq!(progress.envelope,ex.envelope);
    }

    struct NativeWatch { request: Arc<Request>, seen: Vec<NativeProgress>, panic_at: Option<usize>, max: usize }
    impl Write for NativeWatch {
        fn write(&mut self, b:&[u8])->std::io::Result<usize> {
            self.seen.push(self.request.native_progress());
            if self.panic_at==Some(self.seen.len()) { panic!("controlled native uncertainty"); }
            Ok(b.len().min(self.max))
        }
        fn flush(&mut self)->std::io::Result<()> { Ok(()) }
    }
    fn run_native_watch(w:&mut NativeWatch,j:&AgentJob)->Executed {
        let mut last=None;let mut first=||{};let mut body=|_:&str|{};
        let mut io=WriterIo{sleep:&|_|{},last_program_write:&mut last,on_first_byte:&mut first,on_body_written:&mut body};
        execute(w,j,&mut io)
    }

    #[test]
    fn c8i_native_progress_covers_fence_bytes_and_outstanding_call() {
        let j=job(Plan::paste("x").with_submit(0,0),FakeProof::healthy());
        let mut w=NativeWatch{request:j.request.clone(),seen:Vec::new(),panic_at:None,max:2};
        let ex=run_native_watch(&mut w,&j);assert!(matches!(ex.terminal,Terminal::WrittenSubmitted{..}));
        assert!(w.seen.iter().all(|p|p.outstanding&&!p.proven_zero()),"every issued native write is outstanding even before a body byte");
        assert_eq!(w.seen[1].accepted_total,2,"partial opening fence contributes to the accepted lower bound");
        assert_eq!(w.seen[1].envelope.body_offset,0);
        let end=j.request.native_progress();
        assert_eq!(end.accepted_total,j.plan.planned_bytes().len() as u64,"every accepted fence body and control byte stays accounted");
        assert!(!end.outstanding&&!end.unknown&&!end.proven_zero());
        assert_eq!(end.envelope,ex.envelope);
    }

    #[test]
    fn c8i_panic_preserves_known_prefix_and_marks_unknown_completion() {
        let j=job(Plan::raw("abcdef").with_submit(0,0),FakeProof::healthy());
        let mut w=NativeWatch{request:j.request.clone(),seen:Vec::new(),panic_at:Some(2),max:2};
        let ex=run_native_watch(&mut w,&j);
        assert!(matches!(&ex.terminal,Terminal::PartialUnknown { reason,.. } if reason=="writer_panic"),"controlled native panic must return through its actual execution owner");
        assert_eq!(w.seen.len(),2);
        let progress=j.request.native_progress();
        assert!(progress.unknown&&!progress.outstanding&&!progress.proven_zero(),"native unwind leaves sticky unknown completion, never exact zero");
        assert_eq!(progress.accepted_total,2,"known lower bound survives uncertain later completion");
        assert_eq!(ex.envelope.body_offset,2,"panic conversion preserves the last exact prefix");
        assert!(matches!(ex.terminal,Terminal::PartialUnknown{accepted:Accepted{payload:2,..},..}),"panic terminal retains this run's known accepted prefix");
    }

    #[test]
    fn c8i_progress_is_visible_before_body_callback_and_flush() {
        struct FlushWatch { request: Arc<Request>, flushes: usize }
        impl Write for FlushWatch {
            fn write(&mut self,b:&[u8])->std::io::Result<usize>{Ok(b.len())}
            fn flush(&mut self)->std::io::Result<()>{self.flushes+=1;let p=self.request.native_progress();assert_eq!(p.accepted_total,3,"flush sees the accepted body lower bound");assert_eq!(p.envelope.body_offset,3);assert!(p.outstanding,"flush keeps the native call outstanding");Ok(())}
        }
        let j=job(Plan::raw("xyz"),FakeProof::healthy());let mut w=FlushWatch{request:j.request.clone(),flushes:0};let mut last=None;let mut first=||{};let mut callbacks=0;
        let mut callback=|_:&str|{callbacks+=1;let p=j.request.native_progress();assert_eq!(p.accepted_total,3,"accepted bytes are published before body settlement callbacks");assert_eq!(p.envelope.body_offset,3);assert!(!p.outstanding);};
        let mut io=WriterIo{sleep:&|_|{},last_program_write:&mut last,on_first_byte:&mut first,on_body_written:&mut callback};
        let ex=execute(&mut w,&j,&mut io);assert!(matches!(ex.terminal,Terminal::WrittenBody{..}));
        assert_eq!(callbacks,1,"the body callback is exercised");assert_eq!(w.flushes,1,"the flush observer is exercised");
    }

    #[test]
    fn c8i_submit_wrapper_forwards_writer_context() {
        let mut j=job(Plan::raw("abc").with_submit(0,0),FakeProof::healthy());
        let p=Arc::new(ContextProof{rows:Mutex::new(Vec::new()),request:j.request.clone(),plan:j.plan.clone()});
        j.proof=Arc::new(crate::handlers::SubmitWithhold{inner:p.clone(),probe:Arc::new(||false)});
        let mut w=Rec::new();let ex=run_job(&mut w,&j);assert!(matches!(ex.terminal,Terminal::WrittenSubmitted{..}));
        assert_eq!(*p.rows.lock().unwrap(),vec![
            ContextRow{admit:true,phase:None,offset:0,body:0,control:0,payload:0,resumed:false},
            ContextRow{admit:false,phase:Some(Phase::Submit),offset:0,body:3,control:0,payload:3,resumed:false},
        ],"submit wrapper forwards exact admission and final Return context");
        let mut held=job(Plan::raw("abc").with_submit(0,0),FakeProof::healthy());
        let hp=Arc::new(ContextProof{rows:Mutex::new(Vec::new()),request:held.request.clone(),plan:held.plan.clone()});
        held.proof=Arc::new(crate::handlers::SubmitWithhold{inner:hp.clone(),probe:Arc::new(||true)});
        let mut hw=Rec::new();let hx=run_job(&mut hw,&held);
        assert_eq!(hw.bytes(),b"abc","a modal withholds the dependent Return after context forwarding");
        assert!(matches!(hx.terminal,Terminal::PartialExact{reason,..} if reason.contains("submit_withheld")),"wrapper retains its typed modal hold");
        assert_eq!(hp.rows.lock().unwrap().len(),2,"modal path still checks the original receiver context");
    }

    #[test]
    fn c8i_first_native_panic_cannot_be_readmitted_as_zero() {
        let j=job(Plan::raw("x"),FakeProof::healthy());
        let mut w=NativeWatch{request:j.request.clone(),seen:Vec::new(),panic_at:Some(1),max:1};
        let ex=run_native_watch(&mut w,&j);
        assert!(matches!(&ex.terminal,Terminal::PartialUnknown { reason,.. } if reason=="writer_panic"));assert_eq!(w.seen.len(),1);
        let p=j.request.native_progress();
        assert_eq!(p.accepted_total,0,"premise: no exact accepted lower bound");
        assert!(!p.outstanding,"premise: the call unwound");
        assert!(p.unknown&&!p.proven_zero(),"zero lower bound after native uncertainty is not proven zero");
        let mut next=NativeWatch{request:j.request.clone(),seen:Vec::new(),panic_at:None,max:1};
        let result=run_native_watch(&mut next,&j);
        assert!(next.seen.is_empty(),"unknown native completion forbids later readmission even at zero lower bound");
        assert!(matches!(result.terminal,Terminal::PartialUnknown{..}));
        assert!(j.request.native_progress().unknown,"new run initialization never erases unknown completion");
    }

    #[test]
    fn c8i_partial_fence_never_becomes_a_resumable_default_envelope() {
        let mut j=job(Plan::paste("abc").with_submit(0,0),FakeProof::failing_at(0,ProofLoss::Unknown("during-fence".into())));
        let mut w=Rec::new();w.script=VecDeque::from([Step::Short(2)]);
        let first=run_job(&mut w,&j);assert!(matches!(first.terminal,Terminal::PartialUnknown{..}));
        assert_eq!(first.envelope,Envelope::default());assert_eq!(w.bytes().len(),2);
        j.resume=Some(first.envelope);j.proof=FakeProof::healthy();let mut next=Rec::new();let result=run_job(&mut next,&j);
        assert_eq!(next.attempts,0,"an exact native count inside a partial fence grants no resume authority");
        assert!(matches!(result.terminal,Terminal::PartialUnknown{..}));
    }

    #[test]
    fn c8i_resume_rejects_changed_clear_or_submit_plan() {
        let mut j=job(Plan::raw("abcdef").with_submit(0,0),FakeProof::failing_at(0,ProofLoss::Unknown("once".into())));
        let mut w=Rec::new();w.script=VecDeque::from([Step::Short(2)]);
        let first=run_job(&mut w,&j);assert!(matches!(first.terminal,Terminal::PartialExact{..}));
        j.resume=Some(first.envelope);j.proof=FakeProof::healthy();j.plan=j.plan.clone().with_clear(1);
        let mut next=Rec::new();let result=run_job(&mut next,&j);
        assert_eq!(next.attempts,0,"same body and offset do not authorize a changed clear/submit plan");
        assert!(matches!(result.terminal,Terminal::PartialUnknown{..}));
    }

    #[test]
    fn c8i_completed_request_cannot_report_a_second_submission() {
        let mut j=job(Plan::raw("abc").with_submit(0,0),FakeProof::healthy());
        let first=run_job(&mut Rec::new(),&j);assert!(matches!(first.terminal,Terminal::WrittenSubmitted{..}));
        j.resume=Some(first.envelope);let mut next=Rec::new();let result=run_job(&mut next,&j);
        assert_eq!(next.attempts,0);
        assert!(matches!(result.terminal,Terminal::PartialUnknown{..}),"a completed request cannot create another terminal success via empty resume");
    }

    #[test]
    fn c8i_resumed_return_checks_modal_without_a_spacing_wait() {
        let mut j=job(Plan::raw("abc").with_submit(0,0),FakeProof::healthy());
        j.proof=Arc::new(crate::handlers::SubmitWithhold{inner:FakeProof::healthy(),probe:Arc::new(||true)});
        let mut first=Rec::new();let ex=run_job(&mut first,&j);
        assert!(matches!(ex.terminal,Terminal::PartialExact{..}));assert_eq!(first.bytes(),b"abc");
        j.resume=Some(ex.envelope);let mut second=Rec::new();let resumed=run_job(&mut second,&j);
        assert_eq!(second.attempts,0,"resumed Return checks the modal even when no spacing wait invalidates admission");
        assert!(matches!(resumed.terminal,Terminal::PartialExact{reason,..} if reason.contains("submit_withheld")));
    }


    struct BlockedAdmission { entered:std::sync::mpsc::Sender<()>,release:Mutex<std::sync::mpsc::Receiver<()>> }
    impl Proof for BlockedAdmission {
        fn admit(&self)->Result<(),ProofLoss> { self.entered.send(()).unwrap();self.release.lock().unwrap().recv().unwrap();Ok(()) }
        fn recheck(&self,_:Phase)->Result<(),ProofLoss> { Ok(()) }
    }
    fn blocked_job(request:Arc<Request>)->(AgentJob,std::sync::mpsc::Receiver<()>,std::sync::mpsc::Sender<()>) {
        let (entered,seen)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();
        let mut j=job(Plan::raw("owned"),FakeProof::healthy());j.request=request;
        j.proof=Arc::new(BlockedAdmission { entered,release:Mutex::new(wait) });(j,seen,release)
    }
    #[test]
    fn c8operation_rejected_execution_cannot_settle_an_active_run() {
        let req=Request::new("same".into(),7,"test","digest".into());
        let (running,seen,release)=blocked_job(req.clone());
        let worker=std::thread::spawn(move||{let mut w=Rec::new();let ex=run_job(&mut w,&running);(running,ex,w.bytes())});
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let callbacks=Arc::new(std::sync::atomic::AtomicUsize::new(0));let counted=callbacks.clone();
        let mut duplicate=job(Plan::raw("owned"),FakeProof::healthy());duplicate.request=req.clone();
        duplicate.settle=Some(Box::new(move|_|{counted.fetch_add(1,Ordering::SeqCst);}));
        let mut rejected_writer=Rec::new();let rejected=run_job(&mut rejected_writer,&duplicate);
        let refused=matches!(rejected.terminal,Terminal::PartialUnknown{..});
        finish(duplicate,&rejected);
        let unchanged=matches!(req.status(),Status::Writing)&&!req.is_settled()&&req.native.lock().unwrap().running;
        release.send(()).unwrap();let (running,completed,bytes)=worker.join().unwrap();finish(running,&completed);
        assert!(refused&&rejected_writer.bytes().is_empty(),"premise: duplicate run never wrote");
        assert!(unchanged&&callbacks.load(Ordering::SeqCst)==0,"rejected execution settled or ended another active run");
        assert_eq!(bytes,b"owned");assert!(matches!(req.status(),Status::Terminal(Terminal::WrittenBody{..})));
    }
    #[test]
    fn c8operation_late_completion_cannot_settle_a_newer_run() {
        let mut old=job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));let req=old.request.clone();
        let callbacks=Arc::new(std::sync::atomic::AtomicUsize::new(0));let counted=callbacks.clone();
        old.settle=Some(Box::new(move|_|{counted.fetch_add(1,Ordering::SeqCst);}));
        let first=run_job(&mut Rec::new(),&old);assert!(matches!(first.terminal,Terminal::RefusedNoWrite{..}));
        let (running,seen,release)=blocked_job(req.clone());
        let worker=std::thread::spawn(move||{let mut w=Rec::new();let ex=run_job(&mut w,&running);(running,ex,w.bytes())});
        seen.recv_timeout(Duration::from_secs(5)).unwrap();finish(old,&first);
        let unchanged=matches!(req.status(),Status::Writing)&&!req.is_settled()&&req.native.lock().unwrap().running;
        release.send(()).unwrap();let (running,completed,bytes)=worker.join().unwrap();finish(running,&completed);
        assert!(unchanged&&callbacks.load(Ordering::SeqCst)==0,"late completion settled or ended a newer run");
        assert_eq!(bytes,b"owned");assert!(req.is_settled());
    }

    #[test]
    fn c8operation_equal_request_ids_do_not_share_completion_authority() {
        let owner=job(Plan::raw("owned"),FakeProof::healthy());let owner_request=owner.request.clone();
        let ex=run_job(&mut Rec::new(),&owner);
        let mut other=job(Plan::raw("owned"),FakeProof::healthy());let other_request=other.request.clone();
        assert_eq!(owner_request.id,other_request.id);
        let callbacks=Arc::new(std::sync::atomic::AtomicUsize::new(0));let counted=callbacks.clone();
        other.settle=Some(Box::new(move|_|{counted.fetch_add(1,Ordering::SeqCst);}));
        let rejected=finish(other,&ex);
        assert!(matches!(rejected,Terminal::PartialUnknown{..}));
        assert!(!other_request.is_settled()&&matches!(other_request.status(),Status::AcceptedPending)&&callbacks.load(Ordering::SeqCst)==0,
            "equal request ID impersonated the original completion owner");
        assert!(!owner_request.is_settled());
        assert!(matches!(finish(owner,&ex),Terminal::WrittenBody{..}),"wrong-owner attempt consumed the real completion");
    }

    #[test]
    fn c8operation_private_receipt_ignores_mutated_diagnostic_result_fields() {
        let mut owner=job(Plan::raw("owned"),FakeProof::healthy());let req=owner.request.clone();
        let journal=Arc::new(ActualContextJournal { request:req.clone(),plan:owner.plan.clone(),writes:Mutex::new(Vec::new()),outcomes:Mutex::new(Vec::new()),reject:false });
        owner.journal=Some(journal.clone());
        let mut ex=run_job(&mut Rec::new(),&owner);
        ex.terminal=Terminal::WrittenSubmitted { accepted:Accepted::default() };ex.envelope.submitted=true;ex.journaled=false;
        let actual=finish(owner,&ex);
        assert!(matches!(actual,Terminal::WrittenBody { accepted:Accepted { payload:5,control:0,.. } }),
            "mutable diagnostic fields manufactured a submitted writer receipt");
        assert!(!req.native_progress().envelope.submitted);
        let receipts=journal.outcomes.lock().unwrap();assert_eq!(receipts.len(),1);
        assert!(!receipts[0].1.submitted,"mutable diagnostic envelope entered the authoritative receipt");drop(receipts);
        let mut duplicate=job(Plan::raw("owned"),FakeProof::healthy());duplicate.request=req.clone();
        let callbacks=Arc::new(std::sync::atomic::AtomicUsize::new(0));let counted=callbacks.clone();
        duplicate.settle=Some(Box::new(move|_|{counted.fetch_add(1,Ordering::SeqCst);}));
        assert!(matches!(finish(duplicate,&ex),Terminal::PartialUnknown{..}));
        assert_eq!(callbacks.load(Ordering::SeqCst),0,"completed nonce was consumed twice");
        assert!(matches!(req.status(),Status::Terminal(Terminal::WrittenBody{..})));
    }

    #[test]
    fn c8operation_completion_owns_request_until_callback_returns() {
        let mut first=job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));let req=first.request.clone();
        let (entered,seen)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();
        first.settle=Some(Box::new(move|_|{entered.send(()).unwrap();wait.recv().unwrap();}));
        let ex=run_job(&mut Rec::new(),&first);
        let settling=std::thread::spawn(move||finish(first,&ex));seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut next=job(Plan::raw("owned"),FakeProof::healthy());next.request=req.clone();let mut writer=Rec::new();
        let blocked=run_job(&mut writer,&next);let blocked_zero=writer.bytes().is_empty()&&matches!(blocked.terminal,Terminal::PartialUnknown{..});
        finish(next,&blocked);let still_unsettled=!req.is_settled();release.send(()).unwrap();settling.join().unwrap();
        assert!(blocked_zero&&still_unsettled,"a new run entered or settled while the prior completion owned its callback");
        assert!(req.is_settled());
        let mut retry=job(Plan::raw("owned"),FakeProof::healthy());retry.request=req;let mut writer=Rec::new();
        let ex=run_job(&mut writer,&retry);assert!(matches!(finish(retry,&ex),Terminal::WrittenBody{..}));assert_eq!(writer.bytes(),b"owned");
    }

    #[derive(Clone,Debug,PartialEq,Eq)]
    struct JournalContextRow { start:Envelope,resumed:bool,body:String,accepted:Accepted }
    struct ActualContextJournal {
        request:Arc<Request>,plan:Plan,writes:Mutex<Vec<JournalContextRow>>,
        outcomes:Mutex<Vec<(Terminal,Envelope,NativeProgress,bool)>>,reject:bool,
    }
    impl Journal for ActualContextJournal {
        fn writing(&self,_:&Request)->std::io::Result<()> { panic!("legacy writing hook must not be used") }
        fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->std::io::Result<()> { panic!("legacy terminal hook must not be used") }
        fn writing_context(&self,c:&ProofContext<'_>)->std::io::Result<()> {
            assert!(std::ptr::eq(c.request,self.request.as_ref()));assert_eq!(c.plan,&self.plan);
            assert_eq!(c.phase_offset,0);assert_eq!(c.accepted,Accepted::default());
            self.writes.lock().unwrap().push(JournalContextRow { start:c.envelope,resumed:c.resumed,body:c.plan.body().unwrap().into(),accepted:c.accepted });
            if self.reject { Err(std::io::Error::other("fixture context refusal")) } else { Ok(()) }
        }
        fn terminal_receipt(&self,r:&WriterReceipt<'_>)->std::io::Result<()> {
            assert!(std::ptr::eq(r.request(),self.request.as_ref()));assert_eq!(r.plan(),&self.plan);
            self.outcomes.lock().unwrap().push((r.terminal().clone(),r.envelope(),r.native_progress(),r.resumed()));Ok(())
        }
    }
    #[test]
    fn c8operation_journal_receives_actual_plan_envelope_and_original_request_on_resume() {
        let mut first=job(Plan::raw("abcdef").with_submit(0,0),FakeProof::failing_at(0,ProofLoss::Unknown("retry".into())));
        let req=first.request.clone();let plan=first.plan.clone();
        let journal=Arc::new(ActualContextJournal { request:req.clone(),plan:plan.clone(),writes:Mutex::new(Vec::new()),outcomes:Mutex::new(Vec::new()),reject:false });
        first.journal=Some(journal.clone());let mut writer=Rec::new();writer.script=VecDeque::from([Step::Short(2)]);
        let ex=run_job(&mut writer,&first);let partial=ex.envelope;assert!(matches!(finish(first,&ex),Terminal::PartialExact{..}));
        let mut next=job(plan,FakeProof::healthy());next.request=req;next.journal=Some(journal.clone());next.resume=Some(partial);
        let ex=run_job(&mut writer,&next);assert!(matches!(finish(next,&ex),Terminal::WrittenSubmitted{..}));
        assert_eq!(writer.bytes(),b"abcdef\r");
        let writes=journal.writes.lock().unwrap();assert_eq!(writes.len(),2);
        assert_eq!(writes[0].start,Envelope::default());assert!(!writes[0].resumed);
        assert_eq!(writes[1].start,partial);assert!(writes[1].resumed);
        let outcomes=journal.outcomes.lock().unwrap();assert_eq!(outcomes.len(),2);
        assert!(matches!(outcomes[0].0,Terminal::PartialExact{accepted:Accepted{payload:2,..},..}));
        assert_eq!(outcomes[0].2.accepted_total,2);assert!(!outcomes[0].3);
        assert!(matches!(outcomes[1].0,Terminal::WrittenSubmitted{accepted:Accepted{payload:4,control:1,..}}));
        assert_eq!(outcomes[1].2.accepted_total,7);assert!(outcomes[1].3);
        assert_eq!(outcomes[1].1,outcomes[1].2.envelope);
    }
    #[test]
    fn c8operation_context_journal_refusal_is_before_any_native_byte() {
        let mut j=job(Plan::paste("abc").with_clear(0).with_submit(0,0),FakeProof::healthy());
        let journal=Arc::new(ActualContextJournal { request:j.request.clone(),plan:j.plan.clone(),writes:Mutex::new(Vec::new()),outcomes:Mutex::new(Vec::new()),reject:true });
        j.journal=Some(journal.clone());let mut writer=Rec::new();let ex=run_job(&mut writer,&j);
        let terminal=finish(j,&ex);
        assert!(matches!(terminal,Terminal::RefusedNoWrite{reason} if reason.contains("fixture context refusal")));
        assert_eq!(writer.attempts,0,"context barrier refusal wrote fence/control/body bytes");
        assert_eq!(journal.writes.lock().unwrap().len(),1);assert!(journal.outcomes.lock().unwrap().is_empty());
    }
    #[test]
    fn c8operation_failed_partial_receipt_cannot_authorize_resume() {
        let log=Arc::new(Mutex::new(Vec::new()));let mut j=job(Plan::raw("abcdef").with_submit(0,0),FakeProof::failing_at(0,ProofLoss::Unknown("retry".into())));
        j.journal=Some(Arc::new(LogJournal { log,fail_writing:false,fail_terminal:true }));
        let req=j.request.clone();let plan=j.plan.clone();let mut writer=Rec::new();writer.script=VecDeque::from([Step::Short(2)]);
        let ex=run_job(&mut writer,&j);assert!(matches!(ex.terminal,Terminal::PartialExact{..}));let partial=ex.envelope;
        let finished=finish(j,&ex);
        assert!(matches!(finished,Terminal::PartialUnknown{..}),"failed partial receipt retained automatic resume authority");
        let mut retry=job(plan,FakeProof::healthy());retry.request=req;retry.resume=Some(partial);let mut later=Rec::new();
        let rejected=run_job(&mut later,&retry);assert!(matches!(rejected.terminal,Terminal::PartialUnknown{..}));assert!(later.bytes().is_empty());
    }

    #[test]
    fn c8operation_completion_retains_its_original_owed_journal() {
        for replace in [false,true] {
            let mut j=job(Plan::raw("owned"),FakeProof::healthy());
            let make=||Arc::new(ActualContextJournal { request:j.request.clone(),plan:j.plan.clone(),writes:Mutex::new(Vec::new()),outcomes:Mutex::new(Vec::new()),reject:false });
            let original=make();let other=make();j.journal=Some(original.clone());
            let ex=run_job(&mut Rec::new(),&j);
            j.journal=if replace { Some(other.clone()) } else { None };
            assert!(matches!(finish(j,&ex),Terminal::WrittenBody{..}));
            assert_eq!(original.outcomes.lock().unwrap().len(),1,"the owed original terminal journal was lost or replaced");
            assert!(other.outcomes.lock().unwrap().is_empty(),"a replacement journal received unowned writer completion");
        }
    }

    #[test]
    fn c8operation_late_idle_result_cannot_consume_a_newer_completion() {
        let mut old=job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("old".into())));let req=old.request.clone();
        let called=Arc::new(std::sync::atomic::AtomicUsize::new(0));let count=called.clone();
        old.settle=Some(Box::new(move|_|{count.fetch_add(1,Ordering::SeqCst);}));
        let previous=run_job(&mut Rec::new(),&old);
        let mut current=job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("current".into())));current.request=req.clone();
        let latest=run_job(&mut Rec::new(),&current);
        assert!(!req.native.lock().unwrap().running,"premise: completed newer execution is idle");
        let rejected=finish(old,&previous);
        assert!(matches!(rejected,Terminal::PartialUnknown{..})&&called.load(Ordering::SeqCst)==0&&!req.is_settled(),
            "older nonce consumed an idle newer completion");
        assert!(matches!(finish(current,&latest),Terminal::RefusedNoWrite{reason} if reason.contains("current")));
    }
    #[test]
    fn c8operation_same_completion_cannot_reenter_during_settlement() {
        let mut first=job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));let req=first.request.clone();
        let (entered,seen)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();
        first.settle=Some(Box::new(move|_|{entered.send(()).unwrap();wait.recv().unwrap();}));
        let ex=Arc::new(run_job(&mut Rec::new(),&first));let run_ex=ex.clone();
        let settling=std::thread::spawn(move||finish(first,&run_ex));seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let called=Arc::new(std::sync::atomic::AtomicUsize::new(0));let count=called.clone();
        let mut duplicate=job(Plan::raw("owned"),FakeProof::healthy());duplicate.request=req.clone();
        duplicate.settle=Some(Box::new(move|_|{count.fetch_add(1,Ordering::SeqCst);}));
        let rejected=finish(duplicate,&ex);let untouched=!req.is_settled()&&called.load(Ordering::SeqCst)==0;
        release.send(()).unwrap();settling.join().unwrap();
        assert!(matches!(rejected,Terminal::PartialUnknown{..})&&untouched,"same completion ran twice while its first settlement was active");
    }
    #[test]
    fn c8operation_context_barrier_checks_the_job_plan_not_only_its_id() {
        struct PlanJournal(Plan);
        impl Journal for PlanJournal {
            fn writing(&self,_:&Request)->std::io::Result<()>{Ok(())}
            fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->std::io::Result<()>{Ok(())}
            fn writing_context(&self,c:&ProofContext<'_>)->std::io::Result<()> {
                if c.plan!=&self.0 { Err(std::io::Error::other("actual plan differs")) } else { Ok(()) }
            }
        }
        let mut j=job(Plan::raw("B").with_clear(0).with_submit(0,0),FakeProof::healthy());
        j.journal=Some(Arc::new(PlanJournal(Plan::raw("A"))));let mut writer=Rec::new();let ex=run_job(&mut writer,&j);
        assert!(matches!(finish(j,&ex),Terminal::RefusedNoWrite{reason} if reason.contains("actual plan differs"))&&writer.attempts==0,
            "request-only journal bypassed the actual plan mismatch");
    }
    #[test]
    fn c8operation_terminal_journal_panic_returns_unknown_and_releases_completion() {
        struct PanicTerminal;
        impl Journal for PanicTerminal {
            fn writing(&self,_:&Request)->std::io::Result<()> { Ok(()) }
            fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->std::io::Result<()> { panic!("controlled terminal journal panic") }
        }
        let mut j=job(Plan::raw("owned"),FakeProof::healthy());let req=j.request.clone();j.journal=Some(Arc::new(PanicTerminal));
        let ex=run_job(&mut Rec::new(),&j);
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||finish(j,&ex)));
        assert!(matches!(result,Ok(Terminal::PartialUnknown{accepted:Accepted{payload:5,..},..})),
            "terminal journal panic escaped the writer or claimed successful completion");
        assert!(req.is_settled());let native=req.native.lock().unwrap();assert!(!native.settling&&native.completion_key.is_none());
    }
    // Managed request and pending-store bridge regression controls.
    // Synthetic durable edit tokens below are test-only fixtures, not authority issuers.
    fn managed_job(plan:Plan,proof:Arc<FakeProof>)->AgentJob {
        let request=Request::new_managed("managed".into(),7,"fixture","digest".into(),&plan);
        let lease=request.reserve_attempt(&plan,None).unwrap();
        let mut j=job(plan,proof);j.request=request;j.attempt=Some(lease);j
    }

    #[test]
    fn lease_managed_request_keeps_original_monotonic_receipt_age() {
        let received=Instant::now().checked_sub(Duration::from_secs(1)).expect("monotonic fixture prerequisite");
        let req=Request::new_managed_at("age".into(),7,"fixture","digest".into(),received,&Plan::raw("age"));
        assert_eq!(req.created,received,"construction after persistence must not reset request age");
        assert!(matches!(req.reserve_attempt(&Plan::raw("different"),None),Err(LeaseError::NativeConflict)),
            "managed admission must capture exact Plan before the first attempt or edit");
        let lease=req.reserve_attempt(&Plan::raw("age"),None).unwrap();drop(lease);
    }

    struct ManagedStampJournal { stamp:AttemptStamp,starts:std::sync::atomic::AtomicUsize,finishes:std::sync::atomic::AtomicUsize }
    impl Journal for ManagedStampJournal {
        fn writing(&self,_:&Request)->std::io::Result<()> { Err(std::io::Error::other("managed start receipt required")) }
        fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->std::io::Result<()> { Err(std::io::Error::other("managed completion receipt required")) }
        fn writing_attempt(&self,ctx:&ProofContext<'_>,receipt:Option<&AttemptStartReceipt<'_>>)->std::io::Result<()> {
            let start=receipt.ok_or_else(||std::io::Error::other("missing start receipt"))?
                .for_attempt(&self.stamp).map_err(|_|std::io::Error::other("wrong attempt"))?;
            assert!(std::ptr::eq(start.request(),ctx.request));assert_eq!(start.plan(),ctx.plan);
            assert_eq!(start.resume().unwrap_or_default(),ctx.envelope);
            self.starts.fetch_add(1,Ordering::SeqCst);Ok(())
        }
        fn terminal_receipt(&self,receipt:&WriterReceipt<'_>)->std::io::Result<()> {
            let verified=receipt.for_attempt(&self.stamp).map_err(|_|std::io::Error::other("wrong completed attempt"))?;
            assert!(std::ptr::eq(verified.writer().request(),self.stamp.request.as_ref()));
            self.finishes.fetch_add(1,Ordering::SeqCst);Ok(())
        }
    }

    #[test]
    fn lease_start_and_completion_receipts_match_only_the_reserved_object_nonce_and_plan() {
        let mut j=managed_job(Plan::raw("owned"),FakeProof::healthy());
        let stamp=j.attempt.as_ref().unwrap().validate_prepared(&j.request,&j.plan,j.resume).unwrap();
        let journal=Arc::new(ManagedStampJournal{stamp,starts:std::sync::atomic::AtomicUsize::new(0),finishes:std::sync::atomic::AtomicUsize::new(0)});
        j.journal=Some(journal.clone());let mut w=Rec::new();let ex=run_job(&mut w,&j);finish(j,&ex);
        assert_eq!(w.bytes(),b"owned");assert_eq!(journal.starts.load(Ordering::SeqCst),1);assert_eq!(journal.finishes.load(Ordering::SeqCst),1);

        let mut wrong=managed_job(Plan::raw("owned"),FakeProof::healthy());
        let other=Request::new_managed(wrong.request.id.clone(),7,"fixture","digest".into(),&wrong.plan);
        let other_lease=other.reserve_attempt(&wrong.plan,None).unwrap();
        let stamp=other_lease.validate_prepared(&other,&wrong.plan,None).unwrap();
        let journal=Arc::new(ManagedStampJournal{stamp,starts:std::sync::atomic::AtomicUsize::new(0),finishes:std::sync::atomic::AtomicUsize::new(0)});
        wrong.journal=Some(journal.clone());let mut w=Rec::new();let ex=run_job(&mut w,&wrong);
        assert!(matches!(finish(wrong,&ex),Terminal::RefusedNoWrite{..}));
        assert!(w.bytes().is_empty());assert_eq!(journal.starts.load(Ordering::SeqCst),0);assert_eq!(journal.finishes.load(Ordering::SeqCst),0);
    }

    #[test]
    fn lease_prepared_queue_reservation_excludes_edit_and_duplicate_attempt() {
        let j=managed_job(Plan::raw("owned"),FakeProof::healthy());let req=j.request.clone();
        let before=req.native_progress();let plan=j.plan.clone();
        let (tx,rx)=std::sync::mpsc::channel();assert!(tx.send(j).is_ok());
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)));
        assert!(matches!(req.reserve_attempt(&plan,None),Err(LeaseError::Busy)));
        assert_eq!(req.native_progress(),before,"queued reservation cannot manufacture native progress");
        drop(rx.recv_timeout(Duration::from_secs(1)).unwrap()); // never-started job drop
        let edit=req.reserve_zero_write_edit().unwrap();drop(edit);
        let next=req.reserve_attempt(&plan,None).unwrap();drop(next);
    }

    #[test]
    fn lease_managed_job_without_lease_refuses_before_journal_and_native() {
        let mut j=job(Plan::raw("owned"),FakeProof::healthy());
        j.request=Request::new_managed("managed".into(),7,"fixture","digest".into(),&j.plan);
        let log=Arc::new(Mutex::new(Vec::new()));
        j.journal=Some(Arc::new(LogJournal{log:log.clone(),fail_writing:false,fail_terminal:false}));
        let req=j.request.clone();let mut w=Rec::new();let ex=run_job(&mut w,&j);
        assert!(matches!(ex.terminal,Terminal::PartialUnknown{..}));finish(j,&ex);
        assert!(w.bytes().is_empty()&&log.lock().unwrap().is_empty());
        assert_eq!(req.status(),Status::AcceptedPending);assert!(!req.is_settled());
        assert!(req.reserve_zero_write_edit().is_ok());
    }

    #[test]
    fn lease_same_id_different_request_cannot_borrow_attempt() {
        let mut j=managed_job(Plan::raw("owned"),FakeProof::healthy());let actual=j.request.clone();
        let impostor=Request::new_managed(actual.id.clone(),7,"fixture","digest".into(),&j.plan);
        j.request=impostor.clone();let mut w=Rec::new();let ex=run_job(&mut w,&j);
        assert!(matches!(ex.terminal,Terminal::PartialUnknown{..}));finish(j,&ex);
        assert!(w.bytes().is_empty());assert!(actual.native_progress().proven_zero());assert!(impostor.native_progress().proven_zero());
        assert!(actual.reserve_zero_write_edit().is_ok(),"dropping mismatched unstarted job may release only its actual reservation");
        assert!(impostor.reserve_zero_write_edit().is_ok());
    }

    #[test]
    fn lease_exact_reserved_plan_and_resume_are_not_mutable_queue_hints() {
        let mut j=managed_job(Plan::raw("owned"),FakeProof::healthy());
        let original=j.plan.clone();j.plan=j.plan.clone().with_clear(1);
        let mut w=Rec::new();let wrong=run_job(&mut w,&j);
        assert!(matches!(wrong.terminal,Terminal::PartialUnknown{..})&&w.attempts==0);
        j.plan=original;j.resume=Some(Envelope::default());
        let wrong=run_job(&mut w,&j);
        assert!(matches!(wrong.terminal,Terminal::PartialUnknown{..})&&w.attempts==0);
        j.resume=None;let correct=run_job(&mut w,&j);
        assert_eq!(w.bytes(),b"owned");assert!(matches!(finish(j,&correct),Terminal::WrittenBody{..}));
    }

    #[test]
    fn lease_attempt_starts_once_and_spans_execute_to_finish() {
        let j=managed_job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));
        let req=j.request.clone();let plan=j.plan.clone();
        let ex=run_job(&mut Rec::new(),&j);assert!(matches!(ex.terminal,Terminal::RefusedNoWrite{..}));
        assert!(matches!(req.reserve_attempt(&plan,None),Err(LeaseError::Busy)));
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)));
        let mut duplicate=Rec::new();let twice=run_job(&mut duplicate,&j);
        assert!(matches!(twice.terminal,Terminal::PartialUnknown{..})&&duplicate.attempts==0);
        assert!(matches!(finish(j,&ex),Terminal::RefusedNoWrite{..}));
        let lease=req.reserve_attempt(&plan,None).unwrap();drop(lease);
    }

    #[test]
    fn lease_started_handle_drop_does_not_release_unsettled_completion() {
        let j=managed_job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));
        let req=j.request.clone();let plan=j.plan.clone();let ex=run_job(&mut Rec::new(),&j);
        drop(j);drop(ex);
        assert!(matches!(req.reserve_attempt(&plan,None),Err(LeaseError::Busy)));
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)));
        assert!(!req.is_settled(),"dropped completion needs reconciliation, not a fabricated callback");
    }

    #[test]
    fn lease_active_native_panic_and_later_drop_never_release_authority() {
        struct BlockThenPanic { entered:std::sync::mpsc::Sender<()>,release:std::sync::mpsc::Receiver<()> }
        impl Write for BlockThenPanic {
            fn write(&mut self,_:&[u8])->std::io::Result<usize> {
                self.entered.send(()).unwrap();self.release.recv_timeout(Duration::from_secs(5)).unwrap();
                panic!("controlled active native uncertainty")
            }
            fn flush(&mut self)->std::io::Result<()> { Ok(()) }
        }
        let j=managed_job(Plan::raw("owned"),FakeProof::healthy());let req=j.request.clone();let plan=j.plan.clone();
        let (entered,seen)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();
        let worker=std::thread::spawn(move||{
            let mut writer=BlockThenPanic{entered,release:wait};let mut last=None;let mut first=||{};let mut body=|_:&str|{};
            let mut io=WriterIo{sleep:&|_|{},last_program_write:&mut last,on_first_byte:&mut first,on_body_written:&mut body};
            let ex=execute(&mut writer,&j,&mut io);(j,ex)
        });
        seen.recv_timeout(Duration::from_secs(5)).unwrap();assert!(req.native_progress().outstanding);
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)));
        release.send(()).unwrap();let (j,ex)=worker.join().unwrap();
        assert!(req.native_progress().unknown&&!req.native_progress().outstanding);
        assert!(matches!(finish(j,&ex),Terminal::PartialUnknown{..}));drop(ex);
        assert!(matches!(req.reserve_attempt(&plan,None),Err(LeaseError::Busy)),"unknown native completion must retain the managed reservation");
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)));
    }

    #[test]
    fn lease_edit_releases_native_mutex_but_not_authority_during_disk_work() {
        let req=Request::new_managed("edit".into(),7,"fixture","digest".into(),&Plan::raw("x"));
        let mut edit=req.reserve_zero_write_edit().unwrap();
        assert_eq!(edit.validate_held(&req,&Plan::raw("x")),Ok(()));
        let other=Request::new_managed(req.id.clone(),7,"fixture","digest".into(),&Plan::raw("x"));
        assert_eq!(edit.validate_held(&other,&Plan::raw("x")),Err(LeaseError::WrongRequest));
        assert_eq!(edit.validate_held(&req,&Plan::raw("other")),Err(LeaseError::PlanChanged));
        let _stamp=edit.begin_persisting(&Plan::raw("x")).unwrap();
        assert_eq!(edit.validate_held(&req,&Plan::raw("x")),Ok(()));
        assert!(req.native.try_lock().is_ok(),"edit must not hold a native mutex across disk work");
        assert!(matches!(req.reserve_attempt(&Plan::raw("x"),None),Err(LeaseError::Busy)));
        drop(edit); // simulated timeout after disk work may have started
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)),"drop is not proof that a durable edit rolled back");
    }

    #[test]
    fn lease_edit_requires_opaque_matching_durable_resolution_and_preserves_progress() {
        let first=managed_job(Plan::raw("old"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));
        let req=first.request.clone();let old=first.plan.clone();let ex=run_job(&mut Rec::new(),&first);finish(first,&ex);
        let progress=req.native_progress();let mut edit=req.reserve_zero_write_edit().unwrap();
        let changed=old.clone().with_submit(3,4);
        assert!(matches!(edit.begin_persisting(&changed),Err(LeaseError::PlanChanged)));
        let stamp=edit.begin_persisting(&old).unwrap();
        // Explicit synthetic receipt for this isolated writer-layer test; real issuance is store-owned.
        edit.resolve(DurableEditResolution::Committed(DurableEditCommit{stamp})).unwrap();
        assert_eq!(req.native_progress(),progress);assert!(!req.is_cancelled());
        assert!(matches!(req.reserve_attempt(&changed,None),Err(LeaseError::NativeConflict)));
        let lease=req.reserve_attempt(&old,None).unwrap();drop(lease);
        let mut edit=req.reserve_zero_write_edit().unwrap();let stamp=edit.begin_persisting(&old).unwrap();
        edit.resolve(DurableEditResolution::NotWritten(DurableEditNoChange{stamp})).unwrap();
        let lease=req.reserve_attempt(&old,None).unwrap();drop(lease);
    }

    #[test]
    fn lease_edit_same_id_wrong_object_and_cancelled_commit_do_not_mutate_plan() {
        let a=Request::new_managed("same".into(),7,"fixture","digest".into(),&Plan::raw("original"));
        let b=Request::new_managed("same".into(),7,"fixture","digest".into(),&Plan::raw("original"));
        let mut ea=a.reserve_zero_write_edit().unwrap();let sa=ea.begin_persisting(&Plan::raw("original")).unwrap();
        let mut eb=b.reserve_zero_write_edit().unwrap();let _sb=eb.begin_persisting(&Plan::raw("original")).unwrap();
        let result=eb.resolve(DurableEditResolution::Committed(DurableEditCommit{stamp:sa}));
        assert_eq!(result,Err(LeaseError::ResolutionMismatch));assert_eq!(b.native.lock().unwrap().plan,Some(Plan::raw("original")));
        assert!(matches!(b.reserve_attempt(&Plan::raw("wrong"),None),Err(LeaseError::Busy)));
        let c=Request::new_managed("cancelled".into(),7,"fixture","digest".into(),&Plan::raw("original"));
        let mut ec=c.reserve_zero_write_edit().unwrap();let sc=ec.begin_persisting(&Plan::raw("original")).unwrap();c.cancel();
        let result=ec.resolve(DurableEditResolution::Committed(DurableEditCommit{stamp:sc}));
        assert_eq!(result,Err(LeaseError::Cancelled));assert_eq!(c.native.lock().unwrap().plan,Some(Plan::raw("original")));
        assert!(c.is_cancelled());assert!(matches!(c.reserve_zero_write_edit(),Err(LeaseError::Cancelled)));
    }

    #[test]
    fn lease_partial_exact_original_resume_survives_without_allowing_zero_edit() {
        let first=managed_job(Plan::raw("abcdef").with_submit(0,0),FakeProof::failing_at(0,ProofLoss::Unknown("retry".into())));
        let req=first.request.clone();let plan=first.plan.clone();let mut w=Rec::new();w.script=VecDeque::from([Step::Short(2)]);
        let ex=run_job(&mut w,&first);let envelope=ex.envelope;assert!(matches!(finish(first,&ex),Terminal::PartialExact{..}));
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::NativeConflict)));
        assert!(matches!(req.reserve_attempt(&plan.clone().with_clear(1),Some(envelope)),Err(LeaseError::NativeConflict)));
        let lease=req.reserve_attempt(&plan,Some(envelope)).unwrap();
        let mut resumed=job(plan,FakeProof::healthy());resumed.request=req.clone();resumed.resume=Some(envelope);resumed.attempt=Some(lease);
        let ex=run_job(&mut w,&resumed);assert!(matches!(finish(resumed,&ex),Terminal::WrittenSubmitted{..}));
        assert_eq!(w.bytes(),b"abcdef\r");assert_eq!(req.native_progress().accepted_total,7);
    }

    #[test]
    fn lease_completed_body_submit_is_unsupported_until_same_request_bound_submit_hook() {
        let j=managed_job(Plan::raw("body"),FakeProof::healthy());let req=j.request.clone();
        let ex=run_job(&mut Rec::new(),&j);assert!(matches!(finish(j,&ex),Terminal::WrittenBody{..}));
        assert!(matches!(req.reserve_attempt(&Plan::submit_only(0),None),Err(LeaseError::NativeConflict)));
        assert!(matches!(req.reserve_zero_write_edit(),Err(LeaseError::NativeConflict)));
        assert!(req.native.lock().unwrap().final_outcome);
        // No second Request/body/work ID is proposed. Future BoundSubmitLease must
        // authorize one submit slot on THIS Request from durable WrittenBody proof.
    }

    #[test]
    fn lease_legacy_request_job_behavior_remains_unmanaged() {
        let j=job(Plan::raw("legacy"),FakeProof::healthy());assert!(j.attempt.is_none());let req=j.request.clone();
        assert!(matches!(req.reserve_attempt(&j.plan,None),Err(LeaseError::Unmanaged)));
        let mut w=Rec::new();let ex=run_job(&mut w,&j);assert!(matches!(finish(j,&ex),Terminal::WrittenBody{..}));
        assert_eq!(w.bytes(),b"legacy");
    }

    struct PreparedOutcomeJournal {
        stamp:AttemptStamp,refuse_writing:bool,starts:std::sync::atomic::AtomicUsize,
        outcomes:Mutex<Vec<(Terminal,bool,bool,NativeProgress)>>,
    }
    impl Journal for PreparedOutcomeJournal {
        fn writing(&self,_:&Request)->std::io::Result<()> { Err(std::io::Error::other("context required")) }
        fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->std::io::Result<()> { Err(std::io::Error::other("receipt required")) }
        fn writing_attempt(&self,_:&ProofContext<'_>,start:Option<&AttemptStartReceipt<'_>>)->std::io::Result<()> {
            start.unwrap().for_attempt(&self.stamp).map_err(|_|std::io::Error::other("wrong attempt"))?;
            self.starts.fetch_add(1,Ordering::SeqCst);
            if self.refuse_writing { Err(std::io::Error::other("Prepared barrier refusal")) } else { Ok(()) }
        }
        fn terminal_receipt(&self,r:&WriterReceipt<'_>)->std::io::Result<()> {
            r.for_attempt(&self.stamp).map_err(|_|std::io::Error::other("wrong completed attempt"))?;
            self.outcomes.lock().unwrap().push((r.terminal().clone(),r.writing_confirmed(),r.writing_uncertain(),r.native_progress()));
            Ok(()) // recorder only; not a claim of durable source/association closure
        }
    }
    fn prepared_journal(j:&AgentJob,refuse_writing:bool)->Arc<PreparedOutcomeJournal> {
        let stamp=j.attempt.as_ref().unwrap().validate_prepared(&j.request,&j.plan,j.resume).unwrap();
        Arc::new(PreparedOutcomeJournal{stamp,refuse_writing,starts:std::sync::atomic::AtomicUsize::new(0),outcomes:Mutex::new(Vec::new())})
    }

    #[test]
    fn lease_managed_queued_cancel_reaches_original_prepared_terminal_receipt() {
        let mut j=managed_job(Plan::raw("queued"),FakeProof::healthy());let req=j.request.clone();
        let journal=prepared_journal(&j,false);j.journal=Some(journal.clone());
        let (tx,rx)=std::sync::mpsc::channel();assert!(tx.send(j).is_ok());req.cancel();
        let j=rx.recv_timeout(Duration::from_secs(1)).unwrap();let mut w=Rec::new();let ex=run_job(&mut w,&j);
        assert!(matches!(ex.terminal,Terminal::CancelledBeforeWrite));assert!(!ex.journaled);
        assert!(matches!(finish(j,&ex),Terminal::CancelledBeforeWrite));
        assert!(w.bytes().is_empty());assert_eq!(journal.starts.load(Ordering::SeqCst),0);
        let rows=journal.outcomes.lock().unwrap();assert_eq!(rows.len(),1);
        assert!(matches!(rows[0].0,Terminal::CancelledBeforeWrite));assert!(!rows[0].1&&!rows[0].2&&rows[0].3.proven_zero());
    }

    #[test]
    fn lease_managed_writing_refusal_reaches_prepared_terminal_receipt_without_claiming_writing() {
        let mut j=managed_job(Plan::raw("queued"),FakeProof::healthy());
        let journal=prepared_journal(&j,true);j.journal=Some(journal.clone());let mut w=Rec::new();let ex=run_job(&mut w,&j);
        assert!(!ex.journaled);assert!(matches!(finish(j,&ex),Terminal::RefusedNoWrite{..}));
        assert!(w.bytes().is_empty());assert_eq!(journal.starts.load(Ordering::SeqCst),1);
        let rows=journal.outcomes.lock().unwrap();assert_eq!(rows.len(),1);
        assert!(matches!(rows[0].0,Terminal::RefusedNoWrite{..}));assert!(!rows[0].1&&rows[0].2&&rows[0].3.proven_zero(),"an invoked failed Writing barrier has unknown persistence, not proven no-change");
    }


    struct FailingManagedTerminal { barrier_refusal:bool,panic_terminal:bool }
    impl Journal for FailingManagedTerminal {
        fn writing(&self,_:&Request)->std::io::Result<()> {
            if self.barrier_refusal { Err(std::io::Error::other("barrier result may be installed")) } else { Ok(()) }
        }
        fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->std::io::Result<()> {
            if self.panic_terminal { panic!("controlled managed terminal uncertainty"); }
            Err(std::io::Error::other("terminal receipt not stored"))
        }
    }
    #[test]
    fn c8managed_failed_zero_write_receipt_keeps_attempt_exclusion() {
        for barrier_refusal in [false,true] { for panic_terminal in [false,true] {
            let mut j=managed_job(Plan::raw("owned"),FakeProof::refusing(ProofLoss::Unknown("retry".into())));
            let req=j.request.clone();let plan=j.plan.clone();j.journal=Some(Arc::new(FailingManagedTerminal { barrier_refusal,panic_terminal }));
            let mut writer=Rec::new();let ex=run_job(&mut writer,&j);let terminal=finish(j,&ex);
            assert!(matches!(terminal,Terminal::RefusedNoWrite{..})&&req.native_progress().proven_zero()&&writer.bytes().is_empty());
            assert!(matches!(req.reserve_attempt(&plan,None),Err(LeaseError::Busy))
                &&matches!(req.reserve_zero_write_edit(),Err(LeaseError::Busy)),
                "failed zero-write receipt released the managed attempt before durable reconciliation");
        } }
    }
    #[test]
    fn c8managed_failed_cancel_receipt_keeps_prepared_attempt_exclusion() {
        for panic_terminal in [false,true] {
            let mut j=managed_job(Plan::raw("owned"),FakeProof::healthy());let req=j.request.clone();
            j.journal=Some(Arc::new(FailingManagedTerminal { barrier_refusal:false,panic_terminal }));req.cancel();
            let mut writer=Rec::new();let ex=run_job(&mut writer,&j);assert!(matches!(finish(j,&ex),Terminal::CancelledBeforeWrite));
            assert!(writer.bytes().is_empty()&&req.native_progress().proven_zero());
            // Cancellation itself rejects new reservations. Inspect the original owned slot,
            // so that unrelated final/cancelled gates cannot hide loss of the receipt owner.
            assert!(matches!(&req.native.lock().unwrap().operation,Some(ManagedOperation::Attempt { phase:AttemptPhase::Started,.. })),
                "cancelled Prepared outcome lost its unresolved managed receipt owner");
        }
    }
    #[test]
    fn c8owned_append_preserves_original_bytes_fences_clear_and_timing() {
        for plan in [Plan::raw(""),Plan::raw("literal\r\nbody"),Plan::paste("한글\x1b[200~text"),Plan::paste("x").with_clear(17)] {
            let original=plan.clone();let bytes=plan.planned_bytes();let before=plan.exact_view();
            let next=plan.append_owned_submit(Submit { delay_ms:23,min_gap_ms:41 }).expect("an absent Submit must append without rebuilding the body");let after=next.exact_view();
            assert_eq!(next.planned_bytes(),[bytes.as_slice(),b"\r"].concat());assert_eq!(plan,original);
            assert_eq!((after.body,after.control,after.fenced,after.clear_first,after.clear_settle_ms),(before.body,before.control,before.fenced,before.clear_first,before.clear_settle_ms));
            assert_eq!(after.submit,Some(Submit {delay_ms:23,min_gap_ms:41}));
        }
    }
    #[test]
    fn c8owned_append_cannot_replace_submit_or_turn_control_into_a_body() {
        for plan in [Plan::control(vec![27]),Plan::submit_only(0),Plan::raw("x").with_submit(7,11)] {
            let original=plan.clone();assert!(matches!(plan.append_owned_submit(Submit {delay_ms:23,min_gap_ms:41}),Err(LeaseError::PlanChanged)));assert_eq!(plan,original);
        }
    }
    #[test]
    fn c8owned_append_data_does_not_authorize_a_native_plan_change() {
        let plan=Plan::raw("owned");let job=managed_job(plan.clone(),FakeProof::healthy());
        let next=plan.append_owned_submit(Submit {delay_ms:0,min_gap_ms:0}).unwrap();
        drop(job.attempt);
        assert!(job.request.reserve_attempt(&next,None).is_err(),"data helper changed the actual admitted native Plan");
        assert!(job.request.reserve_attempt(&plan,None).is_ok());
    }

}
