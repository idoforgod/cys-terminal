// Durable storage/current-run ownership. Native authority is supplied by actual external adapters.
//! Bounded retention for pending agent requests. Storage only: no retries or replay.
#![allow(dead_code)]
//! Opaque terminal evidence is retained data, NOT validated writer success.
//! Dependencies: std, serde, serde_json. std File::try_lock requires Rust >= 1.89.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError, Weak};
use std::sync::atomic::{AtomicBool,Ordering};

const SCHEMA: u32 = 5;
#[path = "pending_attempts.rs"]
pub(crate) mod attempts;
const DATA: &str = "pending-requests.json";
const TEMP: &str = ".pending-requests.json.tmp";
const LOCK: &str = ".pending-requests.lock";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExactSubmit {
    pub delay_ms: u64,
    pub min_gap_ms: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExactPlan {
    pub clear_first: bool,
    pub clear_settle_ms: u64,
    pub body: Option<String>,
    pub fenced: bool,
    pub control: Option<Vec<u8>>,
    pub submit: Option<ExactSubmit>,
}

impl fmt::Debug for ExactPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExactPlan")
            .field("body_bytes", &self.body.as_ref().map(String::len))
            .field("control_bytes", &self.control.as_ref().map(Vec::len))
            .field("clear_first", &self.clear_first)
            .field("fenced", &self.fenced)
            .field("owns_submit", &self.submit.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EpochStamp {
    pub daemon_instance: u64,
    pub surface_instance: u64,
    pub launch: u64,
    pub conversation: u64,
    pub policy: u64,
    pub binding: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceKind { Direct, Queue, Inbox, Schedule, Feed, Supervisor }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRef {
    pub kind: SourceKind,
    /// Original source IDs, in their original order. Never replaced by a digest.
    pub ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetMode { Surface, Role }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub mode: TargetMode,
    pub surface_id: u64,
    pub role: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingRecord {
    pub request_id: String,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub registration_id: Option<String>,
    pub obligation_id: String,
    pub source: SourceRef,
    pub target: Target,
    pub lineage: RecordLineage,
    pub original_body_digest: String,
    pub received_epoch_ms: u64,
    pub plan: ExactPlan,
}

impl fmt::Debug for PendingRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingRecord")
            .field("request_id", &self.request_id)
            .field("source_kind", &self.source.kind)
            .field("source_id_count", &self.source.ids.len())
            .field("epochs", &self.visible_epochs())
            .field("plan", &self.plan)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct NewSpec {
    pub obligation_id: String,
    pub source: SourceRef,
    pub target: Target,
    pub epochs: EpochStamp,
    pub policy_digest: String,
    pub original_body_digest: String,
    pub received_epoch_ms: u64,
    pub plan: ExactPlan,
}
impl NewSpec {
    fn into_record(self, request_id: String) -> PendingRecord {
        PendingRecord { request_id, registration_id:None, obligation_id:self.obligation_id, source:self.source,
            target:self.target, lineage:RecordLineage::Legacy { epochs:self.epochs,policy_digest:self.policy_digest },
            original_body_digest:self.original_body_digest, received_epoch_ms:self.received_epoch_ms,
            plan:self.plan }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetiredReason {
    WrittenBody, WrittenSubmitted, RefusedNoWrite, CancelledBeforeWrite,
    PartialUnknown, ExplicitUndelivered,
}

/// Opaque evidence is stored exactly as JSON values. The storage layer does NOT
/// validate that a writer succeeded or that the caller supplied a complete receipt.
/// That validation, including request/generation/byte identity, is mandatory at
/// the integration seam. Neither inventory nor Debug exposes this evidence.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalRecord {
    pub reason: RetiredReason,
    pub at_epoch_ms: u64,
    pub evidence: serde_json::Value,
}
impl fmt::Debug for TerminalRecord {
    fn fmt(&self, f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("TerminalRecord").field("reason",&self.reason)
            .field("at_epoch_ms",&self.at_epoch_ms).finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SettlementKind {
    ProducerSettled, ExplicitUndelivered, AlreadyCompleted, Superseded,
    DeliberateNewExecution, DurableOwnershipTransfer,
}

/// Separate consumer receipt. A reference is traceability, not proof by itself:
/// the consumer must verify all source dispositions and outstanding associations.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettlementReceipt {
    pub kind: SettlementKind,
    pub at_epoch_ms: u64,
    pub evidence: serde_json::Value,
}
impl fmt::Debug for SettlementReceipt {
    fn fmt(&self, f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("SettlementReceipt").field("kind",&self.kind)
            .field("at_epoch_ms",&self.at_epoch_ms).finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settled { order:u64, receipt:SettlementReceipt }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    record:PendingRecord, terminal:Option<TerminalRecord>, settled:Option<Settled>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    attempts:Option<attempts::AttemptLedger>,
}
/// Bounds are explicit at the registry-enabled opener, not inferred after admission.
#[derive(Clone,Copy,Debug)]
pub(crate) struct RegistryLimits {
    pub max_catalogs:usize,pub max_registrations:usize,pub max_catalog_bytes:usize,pub max_declarations:usize,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistrationScope {
    pub daemon_instance:u64,pub surface_id:u64,pub surface_instance:u64,pub launch:u64,pub policy:u64,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogRecord { image:String,identity:String }
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationRecord {
    id:String,scope:RegistrationScope,catalog_id:String,contract_key:String,
    origin:crate::receiver::RegistrationOrigin,closed:bool,
}
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryImage {
    next_registration_seq:u64,catalogs:BTreeMap<String,CatalogRecord>,registrations:BTreeMap<String,RegistrationRecord>,
}
impl Default for RegistryImage {
    fn default()->Self { Self { next_registration_seq:1,catalogs:BTreeMap::new(),registrations:BTreeMap::new() } }
}
struct RuntimeIdentity { healthy:AtomicBool }
/// Current-run persisted registration. No Deserialize/Serialize/Clone; Arc clones retain the
/// same authority. Recovered JSON cannot recreate this object or its live loader-owned catalog.
pub(crate) struct RegisteredCatalog {
    record:RegistrationRecord,catalog:Arc<crate::policy_loader::Catalog>,owner:Weak<RuntimeIdentity>,active:AtomicBool,
}
impl fmt::Debug for RegisteredCatalog {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("RegisteredCatalog").field("id",&self.record.id).field("scope",&self.record.scope).finish_non_exhaustive()
    }
}
impl RegisteredCatalog {
    pub fn id(&self)->&str { &self.record.id }
    pub fn scope(&self)->RegistrationScope { self.record.scope }
    pub fn contract_key(&self)->&str { &self.record.contract_key }
    pub fn check_live(&self)->Result<(),StoreError> {
        let owner=self.owner.upgrade().ok_or(StoreError::Missing)?;
        if !owner.healthy.load(Ordering::Acquire) { return Err(StoreError::UncertainCommit); }
        if !self.active.load(Ordering::Acquire) { return Err(StoreError::Conflict); }
        Ok(())
    }
    /// Private immutable config data, not native receiver proof. Admission must check live again.
    pub fn catalog(&self)->Result<&crate::policy_loader::Catalog,StoreError> {
        self.check_live()?;Ok(&self.catalog)
    }
}
#[derive(Clone,Debug)]
pub(crate) struct RegistrationSummary {
    pub id:String,pub scope:RegistrationScope,pub catalog_id:String,pub contract_key:String,
    pub origin:crate::receiver::RegistrationOrigin,pub closed:bool,pub usable_in_this_run:bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot { schema:u32, next_request_seq:u64, next_settle_seq:u64, rows:Vec<Row>, registry:RegistryImage }

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub max_active_records:usize,
    /// Maximum charged row, INCLUDING future terminal and settlement allowances.
    pub max_record_bytes:usize,
    /// Total committed snapshot cap (also covers counter digit-growth headroom).
    pub max_file_bytes:usize,
    /// Maximum encoded entire TerminalRecord, not only its evidence field.
    pub max_terminal_bytes:usize,
    /// Maximum encoded entire Settled, including maximum-width order.
    pub max_settlement_bytes:usize,
    pub max_recent_records:usize,
    pub max_recent_bytes:usize,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum Durability { FileAndDirectorySynced, FileSyncedDirectoryUnavailable }
#[derive(Debug)]
pub(crate) enum StoreError {
    Invalid(&'static str), Locked, Busy, Conflict, Capacity(&'static str),
    Missing, RetirementConflict, SettlementConflict, UncertainCommit,
    Io { stage:&'static str, installed:bool, message:String },
}
#[derive(Debug)]
pub(crate) struct ExecutionPermit { request_id:String }
impl ExecutionPermit { pub fn request_id(&self)->&str { &self.request_id } }
#[derive(Debug)]
pub(crate) enum Admission {
    NewlyStored { permit:ExecutionPermit, durability:Durability },
    ExistingPending, RecoveredOnly, TerminalOnly, SettledOnly, StatusUnavailable,
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(crate) enum Disposition {
    PendingThisRun, RecoveredNeedsReconciliation,
    TerminalProtected { reason:RetiredReason, at_epoch_ms:u64 },
    SettledRecent { reason:RetiredReason, kind:SettlementKind, order:u64, at_epoch_ms:u64 },
}
#[derive(Clone,Debug)]
pub(crate) struct InventoryRow {
    pub request_id:String, pub obligation_id:String, pub source:SourceRef,
    pub target:Target, pub epochs:Option<crate::receiver::Epochs>, pub body_bytes:usize,
    pub control_bytes:usize, pub recovered:bool, pub disposition:Disposition,
}
#[derive(Debug)]
pub(crate) enum PersistResult { Stored(Durability), Identical }
struct State {
    rows:Arc<BTreeMap<String,Row>>, loaded:Arc<BTreeSet<String>>,
    next_request_seq:u64, next_settle_seq:u64, uncertain:bool,
    registry:Arc<RegistryImage>, live_registrations:BTreeMap<RegistrationScope,Arc<RegisteredCatalog>>,
    // Fixed opener census: never extended by current-run registrations. Retirement of these
    // data-only pins is monotonic and cannot grant receiver or request execution authority.
    loaded_registrations:BTreeSet<String>,
    current_requests:BTreeMap<String,Weak<CurrentRequest>>,
}
struct ReadSnapshot {
    rows:Arc<BTreeMap<String,Row>>, loaded:Arc<BTreeSet<String>>, uncertain:bool,
    registry:Arc<RegistryImage>,runtime_registrations:BTreeMap<String,Weak<RegisteredCatalog>>,
}
/// Armed only across persistence and publication. The fallback view is allocated BEFORE
/// any disk mutation so unwinding can quarantine without allocating or panicking in Drop.
/// Normal in-flight readers retain the last published revision; no read waits for fsync.
struct CommitBoundary<'a> {
    state: &'a mut State,
    published: &'a RwLock<Arc<ReadSnapshot>>,
    ambiguous: Option<Arc<ReadSnapshot>>,
    identity:&'a RuntimeIdentity,
}
impl Drop for CommitBoundary<'_> {
    fn drop(&mut self) {
        if let Some(view) = self.ambiguous.take() {
            self.state.uncertain = true;
            self.identity.healthy.store(false,Ordering::Release);
            *self.published.write().unwrap_or_else(|e| e.into_inner()) = view;
        }
    }
}

struct CommitFailure {
    stage: &'static str,
    installed: bool,
    uncertain: bool,
    message: String,
}

/// Private injection seam: tests may fail persistence; production uses RealDisk.
trait Disk: Send + Sync {
    fn replace(&self, root: &Path, bytes: &[u8]) -> Result<Durability, CommitFailure>;
}

struct RealDisk;
impl Disk for RealDisk {
    fn replace(&self, root: &Path, bytes: &[u8]) -> Result<Durability, CommitFailure> {
        let temp = root.join(TEMP);
        let target = root.join(DATA);
        let mut created = false;
        let mut installed = false;
        let mut stage = "create_temp";
        let result = (|| -> io::Result<Durability> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
            let mut file = options.open(&temp)?;
            created = true;
            stage = "write_temp";
            file.write_all(bytes)?;
            stage = "sync_file";
            file.sync_all()?;
            drop(file);
            stage = "rename";
            fs::rename(&temp, &target)?;
            installed = true;
            stage = "sync_directory";
            sync_directory(root)
        })();
        match result {
            Ok(level) => Ok(level),
            Err(error) => {
                // Remove only this transaction's uncommitted staging file.
                let cleanup_failed = created && !installed && temp.exists() && fs::remove_file(&temp).is_err();
                let foreign_staging = !created && temp.exists();
                Err(CommitFailure {
                    stage, installed, uncertain: installed || cleanup_failed || foreign_staging,
                    message: error.to_string(),
                })
            }
        }
    }
}

#[cfg(unix)]
fn sync_directory(root: &Path) -> io::Result<Durability> {
    File::open(root)?.sync_all()?;
    Ok(Durability::FileAndDirectorySynced)
}
#[cfg(not(unix))]
fn sync_directory(_root: &Path) -> io::Result<Durability> {
    Ok(Durability::FileSyncedDirectoryUnavailable)
}

pub(crate) struct PendingStore {
    root:PathBuf, instance:u64, limits:Limits, mutation:Mutex<State>,
    published:RwLock<Arc<ReadSnapshot>>, disk:Arc<dyn Disk>, _process_lock:File,
    registry_limits:Option<RegistryLimits>,identity:Arc<RuntimeIdentity>,
}
impl PendingStore {
    pub fn open(root:&Path, instance:u64, limits:Limits)->Result<Self,StoreError> {
        Self::open_with(root,instance,limits,Arc::new(RealDisk))
    }
    fn open_with(root:&Path, instance:u64, limits:Limits, disk:Arc<dyn Disk>)->Result<Self,StoreError> {
        Self::open_registry_with(root,instance,limits,None,disk)
    }
    pub fn open_with_registry(root:&Path,instance:u64,limits:Limits,registry_limits:RegistryLimits)->Result<Self,StoreError> {
        Self::open_registry_with(root,instance,limits,Some(registry_limits),Arc::new(RealDisk))
    }
    fn open_registry_with(root:&Path,instance:u64,limits:Limits,registry_limits:Option<RegistryLimits>,disk:Arc<dyn Disk>)->Result<Self,StoreError> {
        validate_limits(instance,limits)?;
        if let Some(r)=registry_limits { validate_registry_limits(r,limits)?; }
        let metadata=fs::symlink_metadata(root).map_err(|e|io_error("root",false,e))?;
        if !metadata.file_type().is_dir() { return Err(StoreError::Invalid("root must be a real directory")); }
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode()&0o077!=0 { return Err(StoreError::Invalid("root must be private")); }
        }
        let root=root.canonicalize().map_err(|e|io_error("canonical_root",false,e))?;
        check_regular_private(&root.join(LOCK))?;
        let mut options=OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let process_lock=options.open(root.join(LOCK)).map_err(|e|io_error("open_lock",false,e))?;
        process_lock.try_lock().map_err(|_|StoreError::Locked)?;
        check_regular_private(&root.join(DATA))?;
        let image=load_image(&root.join(DATA),limits,registry_limits)?;
        // Original staging cleanup behavior: committed input validated first.
        check_regular_private(&root.join(TEMP))?;
        if root.join(TEMP).exists() {
            fs::remove_file(root.join(TEMP)).map_err(|e|io_error("remove_uncommitted_temp",false,e))?;
            sync_directory(&root).map_err(|e|io_error("sync_temp_cleanup",false,e))?;
        }
        let loaded: Arc<BTreeSet<String>>=Arc::new(image.rows.keys().cloned().collect());
        let rows=Arc::new(image.rows);let registry=Arc::new(image.registry);
        let loaded_registrations=registry.registrations.keys().cloned().collect();
        let published=Arc::new(ReadSnapshot { rows:rows.clone(),loaded:loaded.clone(),uncertain:false,registry:registry.clone(),runtime_registrations:BTreeMap::new() });
        Ok(Self { root,instance,limits, mutation:Mutex::new(State { rows,loaded,registry,live_registrations:BTreeMap::new(),loaded_registrations,current_requests:BTreeMap::new(),
            next_request_seq:image.next_request_seq,next_settle_seq:image.next_settle_seq,uncertain:false }),
            published:RwLock::new(published),disk,_process_lock:process_lock,registry_limits,
            identity:Arc::new(RuntimeIdentity { healthy:AtomicBool::new(true) }) })
    }

    /// Fresh work only. Callers cannot choose/reuse the new request ID.
    pub fn create_new(&self,spec:NewSpec)->Result<Admission,StoreError> {
        self.create_with_registration(spec,None)
    }
    pub fn create_registered(&self,spec:NewSpec,registration:&Arc<RegisteredCatalog>)->Result<Admission,StoreError> {
        self.create_with_registration(spec,Some(registration))
    }
    fn create_with_registration(&self,spec:NewSpec,registration:Option<&Arc<RegisteredCatalog>>)->Result<Admission,StoreError> {
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        if spec.epochs.daemon_instance!=self.instance {
            return Err(StoreError::Invalid("new admission belongs to another daemon"));
        }
        let seq=state.next_request_seq;
        let next=seq.checked_add(1).ok_or(StoreError::Capacity("request sequence exhausted"))?;
        if seq==0 { return Err(StoreError::Invalid("zero request sequence")); }
        let id=format!("w{:x}-{seq}",self.instance);
        if state.rows.contains_key(&id) { return Err(StoreError::Conflict); }
        let mut record=spec.into_record(id.clone());
        if let Some(registration)=registration {
            self.validate_live_registration(&state,registration)?;
            let scope=registration.scope();
            if record.admission_scope()!=scope { return Err(StoreError::Conflict); }
            record.registration_id=Some(registration.id().to_owned());
        }
        validate_record(&record)?;
        check_raw_record_size(&record,self.limits.max_record_bytes)?;
        let row=Row { record,terminal:None,settled:None,attempts:None };
        checked_row_charge(&row,self.limits)?;
        let mut candidate=(*state.rows).clone();
        candidate.insert(id.clone(),row);
        let next_settle=state.next_settle_seq;
        let mut registry=state.registry.clone();
        let bytes=prune_and_encode(&mut candidate,next,next_settle,self.limits,None,&mut registry,self.registry_limits)?;
        let durability=self.commit(&mut state,candidate,next,next_settle,registry,&bytes)?;
        Ok(Admission::NewlyStored { permit:ExecutionPermit { request_id:id },durability })
    }

    /// Lookup-only. A missing record NEVER falls through to fresh admission.
    pub fn join_existing(&self,record:&PendingRecord)->Result<Admission,StoreError> {
        let state=self.snapshot();
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        let Some(old)=state.rows.get(&record.request_id) else { return Ok(Admission::StatusUnavailable); };
        if &old.record!=record { return Err(StoreError::Conflict); }
        Ok(if old.settled.is_some() { Admission::SettledOnly }
            else if state.loaded.contains(&record.request_id) { Admission::RecoveredOnly }
            else if old.terminal.is_some() { Admission::TerminalOnly }
            else { Admission::ExistingPending })
    }

    /// Durable storage of a caller-validated FINAL result, not a validation oracle.
    /// A resumable PartialExact remains protected/pending; this primitive does not
    /// implement attempt history or replacement of a final result by a later run.
    pub fn record_terminal(&self,id:&str,terminal:TerminalRecord)->Result<PersistResult,StoreError> {
        validate_terminal(&terminal,self.limits)?;
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        let row=state.rows.get(id).ok_or(StoreError::Missing)?;
        if matches!(row.record.lineage,RecordLineage::Protocol { .. }) {
            return Err(StoreError::Invalid("typed protocol writer receipt required"));
        }
        if let Some(old)=&row.terminal {
            return if old==&terminal { Ok(PersistResult::Identical) }
                else { Err(StoreError::RetirementConflict) };
        }
        if row.settled.is_some() { return Err(StoreError::Invalid("settled without terminal")); }
        let mut candidate=(*state.rows).clone();
        candidate.get_mut(id).expect("checked above").terminal=Some(terminal);
        let next_request=state.next_request_seq;
        let next_settle=state.next_settle_seq;
        let mut registry=state.registry.clone();
        let bytes=prune_and_encode(&mut candidate,next_request,next_settle,self.limits,None,&mut registry,self.registry_limits)?;
        self.commit(&mut state,candidate,next_request,next_settle,registry,&bytes).map(PersistResult::Stored)
    }

    /// Consumer-only, AFTER all required durable producer effects/associations.
    /// This is not a writer callback and must not be exposed as a generic RPC.
    pub fn mark_settled(&self,id:&str,receipt:SettlementReceipt)->Result<PersistResult,StoreError> {
        validate_settlement(&receipt,self.limits)?;
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        let row=state.rows.get(id).ok_or(StoreError::Missing)?;
        if matches!(row.record.lineage,RecordLineage::Protocol { .. }) {
            return Err(StoreError::Invalid("typed protocol consumer settlement required"));
        }
        if let Some(old)=&row.settled {
            return if old.receipt==receipt { Ok(PersistResult::Identical) }
                else { Err(StoreError::SettlementConflict) };
        }
        let terminal=row.terminal.as_ref().ok_or(StoreError::Invalid("no durable terminal evidence"))?;
        // A partial/refused result alone does not complete its work obligation.
        validate_settlement_pair(terminal,&receipt)?;
        let order=state.next_settle_seq;
        let next=order.checked_add(1).ok_or(StoreError::Capacity("settlement sequence exhausted"))?;
        if order==0 { return Err(StoreError::Invalid("zero settlement sequence")); }
        let mut candidate=(*state.rows).clone();
        candidate.get_mut(id).expect("checked above").settled=Some(Settled { order,receipt });
        let next_request=state.next_request_seq;
        // This successful commit must contain its own receipt. Later commits may
        // expire it under the bounded history policy; no time guarantee is made.
        let mut registry=state.registry.clone();
        let bytes=prune_and_encode(&mut candidate,next_request,next,self.limits,Some(id),&mut registry,self.registry_limits)?;
        self.commit(&mut state,candidate,next_request,next,registry,&bytes).map(PersistResult::Stored)
    }

    fn try_mutation(&self)->Result<MutexGuard<'_,State>,StoreError> {
        match self.mutation.try_lock() {
            Ok(guard)=>Ok(guard), Err(TryLockError::WouldBlock)=>Err(StoreError::Busy),
            Err(TryLockError::Poisoned(error))=>Ok(error.into_inner()),
        }
    }
    fn commit(&self,state:&mut State,candidate:BTreeMap<String,Row>,next_request:u64,
        next_settle:u64,registry:Arc<RegistryImage>,bytes:&[u8])->Result<Durability,StoreError>
    {
        let ambiguous=Arc::new(ReadSnapshot {
            rows:state.rows.clone(),loaded:state.loaded.clone(),uncertain:true,registry:state.registry.clone(),
            runtime_registrations:state.live_registrations.values().map(|r|(r.id().to_owned(),Arc::downgrade(r))).collect(),
        });
        let mut boundary=CommitBoundary { state,published:&self.published,ambiguous:Some(ambiguous),identity:&self.identity };
        match self.disk.replace(&self.root,bytes) {
            Ok(level)=>{
                boundary.state.rows=Arc::new(candidate);
                boundary.state.registry=registry;
                let retained_rows=&boundary.state.rows;
                boundary.state.current_requests.retain(|id,owner|retained_rows.contains_key(id) && owner.strong_count()>0);
                boundary.state.next_request_seq=next_request;
                boundary.state.next_settle_seq=next_settle;
                boundary.state.loaded=Arc::new(boundary.state.loaded.iter()
                    .filter(|id|boundary.state.rows.contains_key(*id)).cloned().collect());
                self.publish(boundary.state);
                // Keep the ambiguity latch armed until BOTH state and the reader view advance.
                boundary.ambiguous=None;
                Ok(level)
            }
            Err(e)=>{
                if !e.uncertain { boundary.ambiguous=None; }
                Err(StoreError::Io { stage:e.stage,installed:e.installed,message:e.message })
            }
        }
    }

    fn publish(&self,state:&State) {
        let next=Arc::new(ReadSnapshot { rows:state.rows.clone(),loaded:state.loaded.clone(),uncertain:state.uncertain,registry:state.registry.clone(),
            runtime_registrations:state.live_registrations.values().map(|r|(r.id().to_owned(),Arc::downgrade(r))).collect() });
        *self.published.write().unwrap_or_else(|e|e.into_inner())=next;
    }
    fn snapshot(&self)->Arc<ReadSnapshot> {
        self.published.read().unwrap_or_else(|e|e.into_inner()).clone()
    }
    pub fn inventory(&self)->Result<Vec<InventoryRow>,StoreError> {
        let state=self.snapshot();
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        Ok(state.rows.values().map(|row|{
            let r=&row.record;
            let disposition=match (&row.terminal,&row.settled) {
                (Some(t),Some(s))=>Disposition::SettledRecent { reason:t.reason,kind:s.receipt.kind,
                    order:s.order,at_epoch_ms:s.receipt.at_epoch_ms },
                (Some(t),None)=>Disposition::TerminalProtected { reason:t.reason,at_epoch_ms:t.at_epoch_ms },
                (None,_) if state.loaded.contains(&r.request_id)=>Disposition::RecoveredNeedsReconciliation,
                _=>Disposition::PendingThisRun,
            };
            InventoryRow { request_id:r.request_id.clone(),obligation_id:r.obligation_id.clone(),
                source:r.source.clone(),target:r.target.clone(),epochs:r.visible_epochs(),
                body_bytes:r.plan.body.as_ref().map_or(0,String::len),
                control_bytes:r.plan.control.as_ref().map_or(0,Vec::len),
                recovered:state.loaded.contains(&r.request_id),disposition }
        }).collect())
    }
    /// Private recovery inspection only; no executable restore constructor.
    pub fn inspect_exact_record(&self,id:&str)->Result<PendingRecord,StoreError> {
        let state=self.snapshot();
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        state.rows.get(id).map(|r|r.record.clone()).ok_or(StoreError::Missing)
    }
    /// Private bytes for the validation consumer. Ordinary inventory omits them.
    pub fn inspect_terminal(&self,id:&str)->Result<Option<TerminalRecord>,StoreError> {
        let state=self.snapshot();
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        state.rows.get(id).map(|r|r.terminal.clone()).ok_or(StoreError::Missing)
    }
    pub fn inspect_settlement(&self,id:&str)->Result<Option<SettlementReceipt>,StoreError> {
        let state=self.snapshot();
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        state.rows.get(id).map(|r|r.settled.as_ref().map(|s|s.receipt.clone())).ok_or(StoreError::Missing)
    }
}
fn io_error(stage: &'static str, installed: bool, e: io::Error) -> StoreError {
    StoreError::Io { stage, installed, message: e.to_string() }
}

fn check_regular_private(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error("metadata", false, e)),
        Ok(m) => {
            if !m.file_type().is_file() { return Err(StoreError::Invalid("store member is not a regular file")); }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o077 != 0 { return Err(StoreError::Invalid("store member must be private")); }
            }
            Ok(())
        }
    }
}

fn validate_record(r: &PendingRecord) -> Result<(), StoreError> {
    if r.request_id.is_empty() || r.request_id.len() > 256
        || !r.request_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)) {
        return Err(StoreError::Invalid("request ID"));
    }
    if r.obligation_id.is_empty() || r.original_body_digest.is_empty() {
        return Err(StoreError::Invalid("provenance"));
    }
    match &r.lineage {
        RecordLineage::Legacy { epochs,policy_digest } if policy_digest.is_empty()
            || epochs.daemon_instance==0 || epochs.surface_instance==0=>return Err(StoreError::Invalid("legacy provenance")),
        RecordLineage::Legacy { .. }=>{},
        RecordLineage::Protocol { data }=>protocol_shape(r,data)?,
    }
    if r.source.ids.is_empty() || r.source.ids.iter().any(String::is_empty) {
        return Err(StoreError::Invalid("source IDs"));
    }
    if r.target.mode == TargetMode::Role && r.target.role.as_ref().is_none_or(|r| r.is_empty()) {
        return Err(StoreError::Invalid("role target"));
    }
    Ok(())
}

/// Reject an already oversized input before cloning its owned strings/bytes into candidate maps.
/// Encoded JSON is checked separately because escaping and control-byte arrays can expand it.
fn check_raw_record_size(r: &PendingRecord, limit: usize) -> Result<(), StoreError> {
    let lengths = [
        r.request_id.len(), r.obligation_id.len(), encode_limited(&r.lineage,limit)?.len(), r.registration_id.as_ref().map_or(0,String::len),
        r.original_body_digest.len(), r.target.role.as_ref().map_or(0, String::len),
        r.plan.body.as_ref().map_or(0, String::len), r.plan.control.as_ref().map_or(0, Vec::len),
    ];
    let mut total = 0usize;
    for length in lengths.into_iter().chain(r.source.ids.iter().map(String::len)) {
        total = total.checked_add(length).ok_or(StoreError::Capacity("raw bytes"))?;
        if total > limit { return Err(StoreError::Capacity("raw bytes")); }
    }
    Ok(())
}

struct Capped { bytes: Vec<u8>, limit: usize, overflow: bool }
impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.overflow = true;
            return Err(io::Error::other("snapshot byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn encode_limited<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, StoreError> {
    let mut out = Capped { bytes: Vec::new(), limit, overflow: false };
    if serde_json::to_writer(&mut out, value).is_err() {
        return Err(if out.overflow { StoreError::Capacity("encoded bytes") } else { StoreError::Invalid("serialization") });
    }
    Ok(out.bytes)
}

fn validate_limits(instance:u64,l:Limits)->Result<(),StoreError> {
    if instance==0 || l.max_active_records==0 || l.max_recent_records==0
        || l.max_record_bytes<64 || l.max_file_bytes==usize::MAX
        || l.max_record_bytes>l.max_file_bytes || l.max_terminal_bytes<64
        || l.max_settlement_bytes<64 || l.max_terminal_bytes>l.max_record_bytes
        || l.max_settlement_bytes>l.max_record_bytes
        || l.max_recent_bytes<l.max_record_bytes.checked_add(1).ok_or(StoreError::Invalid("limits overflow"))? {
        return Err(StoreError::Invalid("instance or limits"));
    }
    Ok(())
}
fn nonempty_object(value:&serde_json::Value)->bool {
    value.as_object().is_some_and(|m|!m.is_empty())
}
fn validate_terminal(t:&TerminalRecord,l:Limits)->Result<(),StoreError> {
    if !nonempty_object(&t.evidence) { return Err(StoreError::Invalid("terminal evidence must be a nonempty object")); }
    encode_limited(t,l.max_terminal_bytes)?;
    Ok(())
}
fn validate_settlement(r:&SettlementReceipt,l:Limits)->Result<(),StoreError> {
    if !nonempty_object(&r.evidence) { return Err(StoreError::Invalid("settlement evidence must be a nonempty object")); }
    encode_limited(&Settled { order:u64::MAX,receipt:r.clone() },l.max_settlement_bytes)?;
    Ok(())
}
/// One invariant for live mutation AND persisted rows. Stored syntax cannot make
/// unresolved/refused work reclaimable through a transition the live API refuses.
fn validate_settlement_pair(terminal:&TerminalRecord,receipt:&SettlementReceipt)->Result<(),StoreError> {
    if receipt.kind==SettlementKind::ProducerSettled &&
        !matches!(terminal.reason,RetiredReason::WrittenBody|RetiredReason::WrittenSubmitted) {
        return Err(StoreError::Invalid("explicit work disposition required"));
    }
    Ok(())
}
/// Exact encoded row size with reservation for BOTH variable-size future records.
/// The allowances are fixed in Limits BEFORE admission, not sized after the write.
fn checked_row_charge(row:&Row,l:Limits)->Result<usize,StoreError> {
    validate_record(&row.record)?;
    check_raw_record_size(&row.record,l.max_record_bytes)?;
    if let Some(t)=&row.terminal { validate_terminal(t,l)?; }
    if let Some(s)=&row.settled {
        let terminal=row.terminal.as_ref().ok_or(StoreError::Invalid("settled row lacks terminal/order"))?;
        if s.order==0 { return Err(StoreError::Invalid("settled row lacks terminal/order")); }
        validate_settlement_pair(terminal,&s.receipt)?;
        validate_settlement(&s.receipt,l)?;
    }
    attempts::validate_ledger(row,l)?;
    let actual=encode_limited(row,l.max_record_bytes)?.len();
    if row.settled.is_some() { return Ok(actual); }
    #[derive(Serialize)]
    struct Bare<'a> {
        record:&'a PendingRecord, terminal:Option<()>, settled:Option<()>,
        #[serde(skip_serializing_if="Option::is_none")]
        attempts:Option<&'a attempts::AttemptLedger>,
    }
    let bare=encode_limited(&Bare { record:&row.record,terminal:None,settled:None,attempts:row.attempts.as_ref() },l.max_record_bytes)?.len();
    let selected=selection_charge(&row.record,l)?;
    let attempt_extra=attempts::reserved_extra(row,l)?;
    let charged=bare.checked_sub(8).and_then(|n|n.checked_add(l.max_terminal_bytes))
        .and_then(|n|n.checked_add(l.max_settlement_bytes)).and_then(|n|n.checked_add(selected))
        .and_then(|n|n.checked_add(attempt_extra))
        .ok_or(StoreError::Capacity("row reserve overflow"))?;
    if charged>l.max_record_bytes { return Err(StoreError::Capacity("reserved row bytes")); }
    if actual>charged { return Err(StoreError::Invalid("row exceeds reserved form")); }
    Ok(charged)
}
fn encode_image(rows:&BTreeMap<String,Row>,next_request_seq:u64,next_settle_seq:u64,l:Limits,registry:&RegistryImage,registry_limits:Option<RegistryLimits>)
    ->Result<Vec<u8>,StoreError>
{
    validate_registry(registry,rows,registry_limits,false)?;
    #[derive(Serialize)]
    struct Image<'a> { schema:u32,next_request_seq:u64,next_settle_seq:u64,rows:Vec<&'a Row>,registry:&'a RegistryImage }
    let snapshot=Image { schema:SCHEMA,next_request_seq,next_settle_seq,rows:rows.values().collect(),registry };
    let bytes=encode_limited(&snapshot,l.max_file_bytes).map_err(|e|match e {
        StoreError::Capacity("encoded bytes")=>StoreError::Capacity("snapshot bytes"),other=>other })?;
    let mut empty=Snapshot { schema:SCHEMA,next_request_seq,next_settle_seq,rows:Vec::new(),registry:RegistryImage::default() };
    empty.registry.next_registration_seq=registry.next_registration_seq;
    let mut widest=Snapshot { schema:SCHEMA,next_request_seq:u64::MAX,next_settle_seq:u64::MAX,rows:Vec::new(),registry:RegistryImage::default() };
    widest.registry.next_registration_seq=u64::MAX;
    let header=encode_limited(&widest,l.max_file_bytes)?.len()
        .checked_sub(encode_limited(&empty,l.max_file_bytes)?.len()).ok_or(StoreError::Invalid("counter width"))?;
    let mut charge=bytes.len().checked_add(header).ok_or(StoreError::Capacity("snapshot bytes"))?;
    for row in rows.values() {
        let actual=encode_limited(row,l.max_record_bytes)?.len();
        let extra=checked_row_charge(row,l)?.checked_sub(actual).ok_or(StoreError::Invalid("negative charge"))?;
        charge=charge.checked_add(extra).ok_or(StoreError::Capacity("snapshot bytes"))?;
    }
    if charge>l.max_file_bytes { return Err(StoreError::Capacity("snapshot bytes")); }
    Ok(bytes)
}
fn recent_size(rows:&BTreeMap<String,Row>,l:Limits)->Result<(usize,usize),StoreError> {
    let mut count=0usize;
    let mut bytes=0usize;
    for row in rows.values().filter(|r|r.settled.is_some()) {
        count=count.checked_add(1).ok_or(StoreError::Capacity("recent count"))?;
        bytes=bytes.checked_add(encode_limited(row,l.max_record_bytes)?.len())
            .and_then(|n|n.checked_add(1)).ok_or(StoreError::Capacity("recent bytes"))?;
    }
    Ok((count,bytes))
}
fn prune_and_encode(rows:&mut BTreeMap<String,Row>,next_request_seq:u64,next_settle_seq:u64,
    l:Limits,keep_recent:Option<&str>,registry:&mut Arc<RegistryImage>,registry_limits:Option<RegistryLimits>)->Result<Vec<u8>,StoreError>
{
    if rows.values().filter(|r|r.settled.is_none()).count()>l.max_active_records {
        return Err(StoreError::Capacity("active records"));
    }
    for row in rows.values() { checked_row_charge(row,l)?; }
    loop {
        prune_registry(registry,rows);
        let (count,bytes)=recent_size(rows,l)?;
        if count<=l.max_recent_records && bytes<=l.max_recent_bytes {
            match encode_image(rows,next_request_seq,next_settle_seq,l,registry,registry_limits) {
                Ok(bytes)=>return Ok(bytes),
                Err(StoreError::Capacity("snapshot bytes"|"catalog count"|"registration count"))=>{},
                Err(other)=>return Err(other),
            }
        }
        let victim=rows.iter().filter(|(id,r)|r.settled.is_some() && keep_recent!=Some(id.as_str()))
            .min_by_key(|(_,r)|r.settled.as_ref().expect("filtered").order).map(|(id,_)|id.clone());
        let Some(victim)=victim else { return Err(StoreError::Capacity("protected rows plus required settlement")); };
        rows.remove(&victim);
    }
}
struct LoadedImage { rows:BTreeMap<String,Row>,next_request_seq:u64,next_settle_seq:u64,registry:RegistryImage }
fn canonical_sequence(id:&str)->Option<u64> {
    let (instance,seq)=id.strip_prefix('w')?.rsplit_once('-')?;
    if instance.is_empty() || !instance.bytes().all(|b|b.is_ascii_hexdigit())
        || seq.is_empty() || !seq.bytes().all(|b|b.is_ascii_digit()) { return None; }
    seq.parse().ok()
}
fn load_image(path:&Path,l:Limits,registry_limits:Option<RegistryLimits>)->Result<LoadedImage,StoreError> {
    let file=match File::open(path) {
        Ok(f)=>f,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(LoadedImage {
            rows:BTreeMap::new(),next_request_seq:1,next_settle_seq:1,registry:RegistryImage::default() }),
        Err(e)=>return Err(io_error("read_store",false,e)),
    };
    let mut bytes=Vec::new();
    file.take(l.max_file_bytes as u64+1).read_to_end(&mut bytes).map_err(|e|io_error("read_store",false,e))?;
    if bytes.len()>l.max_file_bytes { return Err(StoreError::Capacity("disk bytes")); }
    let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|StoreError::Invalid("persisted JSON"))?;
    let snapshot=match value.get("schema").and_then(serde_json::Value::as_u64) {
        Some(version) if version==u64::from(SCHEMA)=>serde_json::from_value::<Snapshot>(value).map_err(|_|StoreError::Invalid("snapshot shape"))?,
        _=>return Err(StoreError::Invalid("schema")),
    };
    if snapshot.next_request_seq==0 || snapshot.next_settle_seq==0 { return Err(StoreError::Invalid("zero counter")); }
    let mut rows=BTreeMap::new();
    let mut orders=BTreeSet::new();
    for row in snapshot.rows {
        checked_row_charge(&row,l)?;
        if canonical_sequence(&row.record.request_id).is_some_and(|seq|seq>=snapshot.next_request_seq) {
            return Err(StoreError::Invalid("request counter not above retained IDs"));
        }
        if let Some(s)=&row.settled {
            if s.order>=snapshot.next_settle_seq || !orders.insert(s.order) {
                return Err(StoreError::Invalid("settlement order/counter"));
            }
        }
        if rows.insert(row.record.request_id.clone(),row).is_some() { return Err(StoreError::Invalid("duplicate persisted ID")); }
    }
    if rows.values().filter(|r|r.settled.is_none()).count()>l.max_active_records {
        return Err(StoreError::Capacity("disk active records"));
    }
    let (recent_count,recent_bytes)=recent_size(&rows,l)?;
    if recent_count>l.max_recent_records || recent_bytes>l.max_recent_bytes {
        return Err(StoreError::Capacity("disk recent history"));
    }
    // Load never prunes/overwrites a rejected file. All loaded rows are recovery-only.
    validate_registry(&snapshot.registry,&rows,registry_limits,true)?;
    encode_image(&rows,snapshot.next_request_seq,snapshot.next_settle_seq,l,&snapshot.registry,registry_limits)?;
    Ok(LoadedImage { rows,next_request_seq:snapshot.next_request_seq,next_settle_seq:snapshot.next_settle_seq,registry:snapshot.registry })
}
impl PendingStore {
    /// Read-only one-ID status. None means expired OR unknown; it does not prove
    /// zero bytes or authorize a new job. No private Plan/evidence is returned.
    pub fn status(&self,id:&str)->Result<Option<InventoryRow>,StoreError> {
        let state=self.snapshot();
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        Ok(state.rows.get(id).map(|row|{
            let r=&row.record;
            let disposition=match (&row.terminal,&row.settled) {
                (Some(t),Some(s))=>Disposition::SettledRecent { reason:t.reason,kind:s.receipt.kind,
                    order:s.order,at_epoch_ms:s.receipt.at_epoch_ms },
                (Some(t),None)=>Disposition::TerminalProtected { reason:t.reason,at_epoch_ms:t.at_epoch_ms },
                (None,_) if state.loaded.contains(id)=>Disposition::RecoveredNeedsReconciliation,
                _=>Disposition::PendingThisRun,
            };
            InventoryRow { request_id:r.request_id.clone(),obligation_id:r.obligation_id.clone(),
                source:r.source.clone(),target:r.target.clone(),epochs:r.visible_epochs(),
                body_bytes:r.plan.body.as_ref().map_or(0,String::len),
                control_bytes:r.plan.control.as_ref().map_or(0,Vec::len),
                recovered:state.loaded.contains(id),disposition }
        }))
    }
}

fn validate_registry_limits(r:RegistryLimits,l:Limits)->Result<(),StoreError> {
    if r.max_catalogs==0 || r.max_registrations==0 || r.max_declarations==0 || r.max_catalog_bytes<128
        || r.max_catalog_bytes>l.max_file_bytes { return Err(StoreError::Invalid("registry limits")); }
    Ok(())
}
fn raw_digest(text:&str)->String { cys::mission_gate::digest_normalized(text) }
fn validate_catalog_record(id:&str,record:&CatalogRecord,r:RegistryLimits)->Result<(u64,crate::receiver::RegistrationOrigin),StoreError> {
    if record.image.len()>r.max_catalog_bytes || record.identity.len()>r.max_catalog_bytes {
        return Err(StoreError::Capacity("catalog bytes"));
    }
    if id!=format!("c{}",raw_digest(&record.image)) { return Err(StoreError::Invalid("catalog image hash")); }
    let image:serde_json::Value=serde_json::from_str(&record.image).map_err(|_|StoreError::Invalid("catalog JSON"))?;
    let identity:serde_json::Value=serde_json::from_str(&record.identity).map_err(|_|StoreError::Invalid("catalog identity JSON"))?;
    if image["schema"]!="cys-receiver-catalog-v1" { return Err(StoreError::Invalid("catalog schema")); }
    let entries=image["entries"].as_array().ok_or(StoreError::Invalid("catalog entries"))?;
    let rejected=image["rejected"].as_array().ok_or(StoreError::Invalid("catalog rejections"))?;
    if entries.is_empty() || entries.len().saturating_add(rejected.len())>r.max_declarations {
        return Err(StoreError::Capacity("catalog declarations"));
    }
    let mut names=BTreeSet::new();let mut projected=Vec::new();let mut lifecycle=None;
    for entry in entries {
        let policy=&entry["policy"];
        let name=policy["adapter"].as_str().filter(|s|!s.is_empty()).ok_or(StoreError::Invalid("catalog adapter"))?;
        if !names.insert(name) { return Err(StoreError::Invalid("duplicate catalog adapter")); }
        let epoch=policy["policy_epoch"].as_u64().filter(|n|*n>0).ok_or(StoreError::Invalid("catalog policy epoch"))?;
        let origin:crate::receiver::RegistrationOrigin=serde_json::from_value(policy["origin"].clone()).map_err(|_|StoreError::Invalid("catalog origin"))?;
        if lifecycle.is_some_and(|old|old!=(epoch,origin)) { return Err(StoreError::Invalid("catalog lifecycle consistency")); }
        lifecycle=Some((epoch,origin));
        let class:crate::receiver::ProofClass=serde_json::from_value(policy["class"].clone()).map_err(|_|StoreError::Invalid("catalog proof class"))?;
        let risk:Option<crate::receiver::WeakRisk>=serde_json::from_value(policy["weak_risk"].clone()).map_err(|_|StoreError::Invalid("catalog weak risk"))?;
        if (class==crate::receiver::ProofClass::FullRequired)!=risk.is_none() { return Err(StoreError::Invalid("catalog class/risk")); }
        for key in ["declaration","config_digest"] {
            if policy[key].as_str().is_none_or(|s|s.is_empty()) { return Err(StoreError::Invalid("catalog contract field")); }
        }
        let mut row=serde_json::Map::new();
        for key in ["adapter","config_digest","declaration","recipe","class","weak_risk"] {
            row.insert(key.into(),policy.get(key).ok_or(StoreError::Invalid("catalog identity field"))?.clone());
        }
        projected.push(serde_json::Value::Object(row));
    }
    let expected=serde_json::json!({"schema":"cys-receiver-catalog-contract-v1","entries":projected,"rejected":rejected});
    if identity!=expected { return Err(StoreError::Invalid("catalog identity projection")); }
    lifecycle.ok_or(StoreError::Invalid("empty eligible catalog"))
}
fn validate_registry(registry:&RegistryImage,rows:&BTreeMap<String,Row>,limits:Option<RegistryLimits>,deep:bool)->Result<(),StoreError> {
    if registry.next_registration_seq==0 { return Err(StoreError::Invalid("registration counter")); }
    let nonempty=!registry.catalogs.is_empty() || !registry.registrations.is_empty();
    let Some(limits)=limits else {
        if nonempty || rows.values().any(|r|r.record.registration_id.is_some()) {
            return Err(StoreError::Invalid("registry limits required"));
        }
        return Ok(());
    };
    if registry.catalogs.len()>limits.max_catalogs { return Err(StoreError::Capacity("catalog count")); }
    if registry.registrations.len()>limits.max_registrations { return Err(StoreError::Capacity("registration count")); }
    let mut lifecycle=BTreeMap::new();
    if deep { for (id,catalog) in &registry.catalogs { lifecycle.insert(id,validate_catalog_record(id,catalog,limits)?); } }
    for (id,record) in &registry.registrations {
        let Some((instance,sequence))=id.strip_prefix('r').and_then(|s|s.split_once('-')) else { return Err(StoreError::Invalid("registration ID")); };
        let sequence=sequence.parse::<u64>().map_err(|_|StoreError::Invalid("registration sequence"))?;
        if id!=&record.id || id!=&format!("r{:x}-{sequence}",record.scope.daemon_instance)
            || instance!=format!("{:x}",record.scope.daemon_instance)
            || sequence==0 || sequence>=registry.next_registration_seq
            || record.scope.daemon_instance==0 || record.scope.surface_instance==0 || record.scope.policy==0 {
            return Err(StoreError::Invalid("registration identity/scope"));
        }
        let catalog=registry.catalogs.get(&record.catalog_id).ok_or(StoreError::Invalid("registration catalog reference"))?;
        if record.contract_key!=raw_digest(&catalog.identity) { return Err(StoreError::Invalid("registration contract reference")); }
        if deep && lifecycle.get(&record.catalog_id)!=Some(&(record.scope.policy,record.origin)) {
            return Err(StoreError::Invalid("registration catalog lifecycle"));
        }
    }
    for row in rows.values() {
        if let Some(id)=&row.record.registration_id {
            let registered=registry.registrations.get(id).ok_or(StoreError::Invalid("request registration reference"))?;
            if row.record.admission_scope()!=registered.scope {
                return Err(StoreError::Invalid("request registration scope"));
            }
        }
        protocol_registry_reference(&row.record,registry)?;
    }
    Ok(())
}
fn prune_registry(registry:&mut Arc<RegistryImage>,rows:&BTreeMap<String,Row>) {
    let referenced:BTreeSet<&str>=rows.values().filter_map(|r|r.record.registration_id.as_deref()).collect();
    let dead:Vec<String>=registry.registrations.iter().filter(|(id,r)|r.closed && !referenced.contains(id.as_str()))
        .map(|(id,_)|id.clone()).collect();
    let catalog_refs:BTreeSet<&str>=registry.registrations.iter().filter(|(id,_)|!dead.contains(id))
        .map(|(_,r)|r.catalog_id.as_str()).collect();
    let orphan:Vec<String>=registry.catalogs.keys().filter(|id|!catalog_refs.contains(id.as_str())).cloned().collect();
    if dead.is_empty() && orphan.is_empty() { return; }
    let image=Arc::make_mut(registry);
    for id in dead { image.registrations.remove(&id); }
    for id in orphan { image.catalogs.remove(&id); }
}
impl PendingStore {
    fn validate_live_registration(&self,state:&State,cap:&Arc<RegisteredCatalog>)->Result<(),StoreError> {
        cap.check_live()?;
        let owner=cap.owner.upgrade().ok_or(StoreError::Missing)?;
        if !Arc::ptr_eq(&owner,&self.identity)
            || !state.live_registrations.get(&cap.record.scope).is_some_and(|live|Arc::ptr_eq(live,cap))
            || !state.registry.registrations.get(cap.id()).is_some_and(|r|r==&cap.record && !r.closed) {
            return Err(StoreError::Conflict);
        }
        Ok(())
    }
    /// Persist exact policy data AND its registration pin atomically before returning a current-run
    /// capability. Same-contract calls preserve the first origin, epoch, ID and immutable catalog.
    pub fn register_catalog(&self,scope:RegistrationScope,catalog:Arc<crate::policy_loader::Catalog>)->Result<Arc<RegisteredCatalog>,StoreError> {
        let limits=self.registry_limits.ok_or(StoreError::Invalid("registry disabled"))?;
        if scope.daemon_instance!=self.instance || scope.surface_instance==0 || scope.policy==0 {
            return Err(StoreError::Invalid("registration scope"));
        }
        if catalog.declaration_count()>limits.max_declarations { return Err(StoreError::Capacity("catalog declarations")); }
        let image=String::from_utf8(encode_limited(&catalog.for_private_persistence(),limits.max_catalog_bytes)?)
            .map_err(|_|StoreError::Invalid("catalog UTF-8"))?;
        let identity=String::from_utf8(encode_limited(&catalog.private_contract_identity(),limits.max_catalog_bytes)?)
            .map_err(|_|StoreError::Invalid("catalog identity UTF-8"))?;
        let contract_key=raw_digest(&identity);let catalog_id=format!("c{}",raw_digest(&image));
        let catalog_record=CatalogRecord { image,identity };validate_catalog_record(&catalog_id,&catalog_record,limits)?;
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        if let Some(existing)=state.live_registrations.get(&scope) {
            existing.check_live()?;
            if existing.record.contract_key==contract_key { return Ok(existing.clone()); }
            return Err(StoreError::Conflict); // configuration change needs an explicit new policy scope
        }
        let origin=catalog.entries().next().ok_or(StoreError::Invalid("empty eligible catalog"))?.1.policy.origin;
        if catalog.entries().any(|(_,p)|p.policy.policy_epoch!=scope.policy || p.policy.origin!=origin) {
            return Err(StoreError::Invalid("catalog lifecycle scope"));
        }
        let mut registry=(*state.registry).clone();let seq=registry.next_registration_seq;
        registry.next_registration_seq=seq.checked_add(1).ok_or(StoreError::Capacity("registration sequence exhausted"))?;
        let id=format!("r{:x}-{seq}",self.instance);
        if registry.registrations.contains_key(&id) { return Err(StoreError::Conflict); }
        let record=RegistrationRecord { id:id.clone(),scope,catalog_id:catalog_id.clone(),contract_key,origin,closed:false };
        registry.catalogs.insert(catalog_id,catalog_record);registry.registrations.insert(id,record.clone());
        let mut registry=Arc::new(registry);let mut rows=(*state.rows).clone();
        let nr=state.next_request_seq;let ns=state.next_settle_seq;
        let bytes=prune_and_encode(&mut rows,nr,ns,self.limits,None,&mut registry,self.registry_limits)?;
        self.commit(&mut state,rows,nr,ns,registry,&bytes)?;
        let capability=Arc::new(RegisteredCatalog { record,catalog,owner:Arc::downgrade(&self.identity),active:AtomicBool::new(true) });
        state.live_registrations.insert(scope,capability.clone());self.publish(&state);Ok(capability)
    }
    /// Revoke locally before persistence. A failed close never reactivates authority; retrying the
    /// same close is permitted. The lifecycle owner must separately retire its native tokens.
    pub fn close_registration(&self,cap:&Arc<RegisteredCatalog>)->Result<PersistResult,StoreError> {
        let owner=cap.owner.upgrade().ok_or(StoreError::Missing)?;
        if !Arc::ptr_eq(&owner,&self.identity) { return Err(StoreError::Conflict); }
        // Busy persistence also leaves authority revoked. Existing admissions still have to
        // recheck this capability before use; a stored row is not permission to write.
        cap.active.store(false,Ordering::Release);
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        let Some(record)=state.registry.registrations.get(cap.id()) else {
            return if !cap.active.load(Ordering::Acquire) { Ok(PersistResult::Identical) } else { Err(StoreError::Missing) };
        };
        if record.closed { cap.active.store(false,Ordering::Release);return Ok(PersistResult::Identical); }
        if !state.live_registrations.get(&cap.record.scope).is_some_and(|live|Arc::ptr_eq(live,cap)) {
            return Err(StoreError::Conflict);
        }
        let mut registry=(*state.registry).clone();registry.registrations.get_mut(cap.id()).unwrap().closed=true;
        let mut registry=Arc::new(registry);let mut rows=(*state.rows).clone();
        let nr=state.next_request_seq;let ns=state.next_settle_seq;
        let bytes=prune_and_encode(&mut rows,nr,ns,self.limits,None,&mut registry,self.registry_limits)?;
        let durability=self.commit(&mut state,rows,nr,ns,registry,&bytes)?;
        state.live_registrations.remove(&cap.record.scope);self.publish(&state);Ok(PersistResult::Stored(durability))
    }
    /// Explicit recovery retirement of a data-only registration from this opener's snapshot.
    /// IDs created in this run cannot use this route. Referenced catalogs remain protected by
    /// their request rows; this does not reconcile those requests or reconstruct live authority.
    pub fn retire_recovered_registration(&self,id:&str)->Result<PersistResult,StoreError> {
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(StoreError::UncertainCommit); }
        if !state.loaded_registrations.contains(id) { return Err(StoreError::Conflict); }
        let Some(record)=state.registry.registrations.get(id) else { return Ok(PersistResult::Identical); };
        if record.closed { return Ok(PersistResult::Identical); }
        let mut registry=(*state.registry).clone();registry.registrations.get_mut(id).unwrap().closed=true;
        let mut registry=Arc::new(registry);let mut rows=(*state.rows).clone();
        let nr=state.next_request_seq;let ns=state.next_settle_seq;
        let bytes=prune_and_encode(&mut rows,nr,ns,self.limits,None,&mut registry,self.registry_limits)?;
        let durability=self.commit(&mut state,rows,nr,ns,registry,&bytes)?;
        Ok(PersistResult::Stored(durability))
    }
    /// Safe data-only inventory. Reopening never inserts entries into live_registrations.
    pub fn registrations(&self)->Result<Vec<RegistrationSummary>,StoreError> {
        let state=self.snapshot();if state.uncertain { return Err(StoreError::UncertainCommit); }
        Ok(state.registry.registrations.values().map(|r|RegistrationSummary { id:r.id.clone(),scope:r.scope,
            catalog_id:r.catalog_id.clone(),contract_key:r.contract_key.clone(),origin:r.origin,closed:r.closed,
            usable_in_this_run:!r.closed && state.runtime_registrations.get(&r.id).and_then(Weak::upgrade)
                .is_some_and(|cap|cap.check_live().is_ok()) }).collect())
    }
    /// Private recovery DATA only, not a Catalog or current-run registration capability.
    pub fn inspect_private_catalog(&self,id:&str)->Result<serde_json::Value,StoreError> {
        let state=self.snapshot();if state.uncertain { return Err(StoreError::UncertainCommit); }
        let record=state.registry.catalogs.get(id).ok_or(StoreError::Missing)?;
        serde_json::from_str(&record.image).map_err(|_|StoreError::Invalid("catalog JSON"))
    }
}

// Durable ingress/initial selection under actual store-issued current ownership.
// Attempts live in pending_attempts.rs; submit/rebind/source settlement are separate
// runtime integration steps and must not be inferred from stored data.
use crate::agent_write::{EditStamp,LeaseError,Plan,Request,ZeroWriteEditLease};
use crate::input_intent::{InputIntent,TargetKind};
use crate::receiver::{Epochs,ProofClass,WeakRisk};
use std::time::{Duration,Instant};

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case")]
pub(crate) enum CallerRecord {
    Surface { id:u64 }, Claimed { id:u64 }, Process { parent_pid:u32,parent_birth_us:u64 },
}
impl From<&crate::submit_assoc::CallerKey> for CallerRecord {
    fn from(c:&crate::submit_assoc::CallerKey)->Self { match c {
        crate::submit_assoc::CallerKey::Surface(id)=>Self::Surface { id:*id },
        crate::submit_assoc::CallerKey::Claimed(id)=>Self::Claimed { id:*id },
        crate::submit_assoc::CallerKey::Process { parent_pid,parent_birth_us }=>Self::Process { parent_pid:*parent_pid,parent_birth_us:*parent_birth_us },
    } }
}
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IngressScope {
    pub socket_path:PathBuf,pub daemon_instance:u64,pub surface_instance:u64,
    pub launch:u64,pub conversation:u64,
}
impl fmt::Debug for IngressScope {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("IngressScope").field("daemon_instance",&self.daemon_instance)
            .field("surface_instance",&self.surface_instance).field("launch",&self.launch)
            .field("conversation",&self.conversation).finish_non_exhaustive()
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitialCatalog {
    pub registration_id:String,pub catalog_id:String,pub contract_key:String,pub policy_epoch:u64,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectedLineage {
    pub catalog_id:String,pub adapter:String,pub config_digest:String,pub epochs:Epochs,
    pub class:ProofClass,pub weak_risk:Option<WeakRisk>,pub publication_ref:String,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case")]
pub(crate) enum SubmitContract {
    OwnedAtAdmission,SelfContainedInBody,AwaitingAssociation { window_ms:u64 },NotApplicable,
    BoundToCompletedBody { body_request_id:String,body_lineage:SelectedLineage },
}
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImmutableIngress {
    pub intent:InputIntent,pub target_kind:TargetKind,pub scope:IngressScope,pub origin:String,
    pub caller:Option<CallerRecord>,pub submit:SubmitContract,
}
impl fmt::Debug for ImmutableIngress {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("ImmutableIngress").field("intent",&self.intent)
            .field("target_kind",&self.target_kind).field("scope",&self.scope).finish_non_exhaustive()
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtocolData {
    pub ingress:ImmutableIngress,pub initial:InitialCatalog,
    pub selected:Option<SelectedLineage>,pub revision:u64,
    /// Encoded selected-lineage allowance declared BEFORE durable admission.
    pub selected_reserve:usize,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="kind",rename_all="snake_case")]
pub(crate) enum RecordLineage {
    Legacy { epochs:EpochStamp,policy_digest:String },
    Protocol { data:ProtocolData },
}

#[derive(Clone,Copy,Debug)]
pub(crate) struct ProtocolLimits { pub max_selected_lineage_bytes:usize }
/// No caller-selected writer ID. Plan is the EXISTING executable Plan, already
/// sanitized at its real ingress constructor; storage never re-sanitizes it.
pub(crate) struct NewProtocolSpec {
    pub obligation_id:String,pub source:SourceRef,pub target:Target,
    pub ingress:ImmutableIngress,pub original_body_digest:String,
    pub received_epoch_ms:u64,pub received_at:Instant,pub plan:Plan,pub origin:&'static str,
}
impl fmt::Debug for NewProtocolSpec {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("NewProtocolSpec").field("source_kind",&self.source.kind)
            .field("source_count",&self.source.ids.len()).field("ingress",&self.ingress).finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub(crate) enum ProtocolError {
    Store(StoreError),Authority(String),RecoveredOnly,RevisionChanged,Conflict,
    Missing,Inactive,Unsupported,NeedsExistingRequest,SelectionRequired,AttemptOutstanding,Lease(LeaseError),
}
impl From<StoreError> for ProtocolError { fn from(e:StoreError)->Self { Self::Store(e) } }
impl From<LeaseError> for ProtocolError { fn from(e:LeaseError)->Self { Self::Lease(e) } }

/// REQUIRED REAL ingress seam. Implement only on a consumed capability minted by
/// the actual authenticated ingress/classifier/control/lifecycle route. No
/// implementation or public data-to-capability constructor is supplied here.
/// It must bind exact Plan, intent, target/socket/scope, source IDs, caller and age.
/// Human/protocol enums or a JSON `authorized` flag are not this capability.
pub(crate) trait IngressAuthority {
    fn validate_and_consume(self,spec:&NewProtocolSpec,registration:&RegisteredCatalog)->Result<(),ProtocolError>;
}
/// REQUIRED REAL controller seam. Only an actual published full/weak binding's
/// factory may implement this. It must bind CurrentRequest, exact expected
/// revision, initial catalog, actual selected contract/epochs and publication.
/// It performs no new discovery here. Saved JSON/IDs cannot implement it.
pub(crate) trait InitialSelectionAuthority {
    fn validate_and_consume(self,request:&Arc<Request>,plan:&Plan,ingress:&ImmutableIngress,
        initial:&InitialCatalog,expected_revision:u64)->Result<SelectedLineage,ProtocolError>;
}
/// These two non-Clone, non-deserializable receipts are minted ONLY by this
/// module's actual selection CAS outcomes. EditStamp stays writer-owned/opaque.
/// Consuming a receipt releases exclusion only; it does not authorize a write.
pub(crate) struct DurableSelectionCommit {
    owner:Weak<RuntimeIdentity>,current:Arc<CurrentRequest>,stamp:EditStamp,
    before:u64,after:u64,durability:Durability,
}
pub(crate) struct DurableSelectionNotWritten {
    owner:Weak<RuntimeIdentity>,current:Arc<CurrentRequest>,stamp:EditStamp,unchanged:u64,
}
fn check_resolution_owner(owner:&Weak<RuntimeIdentity>,current:&Arc<CurrentRequest>,
    request:&Arc<Request>,plan:&Plan)->Result<(),ProtocolError>
{
    let owner=owner.upgrade().ok_or(ProtocolError::Inactive)?;
    let admitted=current.owner.upgrade().ok_or(ProtocolError::Inactive)?;
    if !Arc::ptr_eq(&owner,&admitted) || !owner.healthy.load(Ordering::Acquire)
        || !Arc::ptr_eq(&current.request,request) || current.plan.as_ref()!=plan {
        return Err(ProtocolError::Conflict);
    }
    Ok(())
}
impl DurableSelectionCommit {
    pub fn durability(&self)->Durability { self.durability }
    /// Called only by ZeroWriteEditLease::resolve_selection (crossmodule patch).
    /// The actual lease additionally compares this opaque stamp's nonce/Plan.
    pub fn consume(self,request:&Arc<Request>,plan:&Plan,expected:u64)
        ->Result<(EditStamp,u64),ProtocolError>
    {
        check_resolution_owner(&self.owner,&self.current,request,plan)?;
        if self.before!=expected || expected.checked_add(1)!=Some(self.after) {
            return Err(ProtocolError::RevisionChanged);
        }
        Ok((self.stamp,self.after))
    }
}
impl DurableSelectionNotWritten {
    pub fn consume(self,request:&Arc<Request>,plan:&Plan,expected:u64)
        ->Result<EditStamp,ProtocolError>
    {
        check_resolution_owner(&self.owner,&self.current,request,plan)?;
        if self.unchanged!=expected { return Err(ProtocolError::RevisionChanged); }
        Ok(self.stamp)
    }
}

/// Runtime ownership, NOT execution/native/ingress authority. No Clone/Deserialize
/// or public fields; only the successful durable admission path below constructs
/// it. Arc clones are the same handle, never a second Request with the same ID.
pub(crate) struct CurrentRequest {
    owner:Weak<RuntimeIdentity>,request:Arc<Request>,plan:Arc<Plan>,
    initial_registration:Arc<RegisteredCatalog>,association_deadline:Option<Instant>,
}
impl fmt::Debug for CurrentRequest {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("CurrentRequest").field("id",&self.request.id).finish_non_exhaustive()
    }
}
impl CurrentRequest {
    pub fn id(&self)->&str { &self.request.id }
    pub fn request(&self)->&Arc<Request> { &self.request }
    pub fn plan(&self)->&Plan { &self.plan }
    pub fn received_at(&self)->Instant { self.request.created }
    /// Original association deadline; data only, never a grant to attach Return.
    pub fn association_deadline(&self)->Option<Instant> { self.association_deadline }
}
/// Dependency: root's read-only Plan::exact_view() accessor. It borrows the exact private
/// fields. planned_bytes cannot reconstruct spacing, empty options or ownership.
fn retained_plan(plan:&Plan)->ExactPlan {
    let view=plan.exact_view();
    ExactPlan { clear_first:view.clear_first,clear_settle_ms:view.clear_settle_ms,
        body:view.body.map(str::to_owned),fenced:view.fenced,control:view.control.map(|bytes|bytes.to_vec()),
        submit:view.submit.map(|s|ExactSubmit { delay_ms:s.delay_ms,min_gap_ms:s.min_gap_ms }) }
}

fn plan_matches(exact:&ExactPlan,plan:&Plan)->bool {
    let view=plan.exact_view();
    exact.clear_first==view.clear_first && exact.clear_settle_ms==view.clear_settle_ms
        && exact.body.as_deref()==view.body && exact.fenced==view.fenced
        && exact.control.as_deref()==view.control && exact.submit.as_ref().map(|s|(s.delay_ms,s.min_gap_ms))
            ==view.submit.map(|s|(s.delay_ms,s.min_gap_ms))
}

impl PendingRecord {
    fn visible_epochs(&self)->Option<Epochs> { match &self.lineage {
        RecordLineage::Legacy { epochs:e,.. }=>Some(Epochs { daemon_instance:e.daemon_instance,surface_instance:e.surface_instance,
            launch:e.launch,conversation:e.conversation,policy:e.policy,binding:e.binding }),
        RecordLineage::Protocol { data }=>data.selected.as_ref().map(|s|s.epochs),
    } }
    fn admission_scope(&self)->RegistrationScope { match &self.lineage {
        RecordLineage::Legacy { epochs:e,.. }=>RegistrationScope { daemon_instance:e.daemon_instance,
            surface_id:self.target.surface_id,surface_instance:e.surface_instance,launch:e.launch,policy:e.policy },
        RecordLineage::Protocol { data }=>RegistrationScope { daemon_instance:data.ingress.scope.daemon_instance,
            surface_id:self.target.surface_id,surface_instance:data.ingress.scope.surface_instance,
            launch:data.ingress.scope.launch,policy:data.initial.policy_epoch },
    } }
}
fn protocol_shape(r:&PendingRecord,p:&ProtocolData)->Result<(),StoreError> {
    if p.revision==0 || p.selected_reserve<128 || p.ingress.scope.daemon_instance==0
        || p.ingress.scope.surface_instance==0 || !p.ingress.scope.socket_path.is_absolute()
        || p.initial.registration_id.is_empty() || p.initial.catalog_id.is_empty()
        || p.ingress.origin.is_empty() || p.initial.contract_key.is_empty() || p.initial.policy_epoch==0
        || r.registration_id.as_deref()!=Some(p.initial.registration_id.as_str()) {
        return Err(StoreError::Invalid("protocol ingress identity"));
    }
    match (p.ingress.intent,p.ingress.target_kind) {
        (InputIntent::AgentText|InputIntent::AgentSubmit|InputIntent::AgentControl,TargetKind::Agent)=>{},
        (InputIntent::LifecycleLaunch,TargetKind::Shell)=>{},
        _=>return Err(StoreError::Invalid("intent outside durable automatic ingress")),
    }
    if matches!(p.ingress.intent,InputIntent::AgentText|InputIntent::LifecycleLaunch)
        && (r.plan.body.is_none() || r.plan.control.is_some()) {
        return Err(StoreError::Invalid("automatic text/launch plan shape"));
    }
    if p.ingress.intent==InputIntent::AgentSubmit &&
        (r.plan.body.is_some() || r.plan.control.is_some() || r.plan.submit.is_none() || r.plan.clear_first) {
        return Err(StoreError::Invalid("submit plan shape"));
    }
    if p.ingress.intent==InputIntent::AgentControl && (r.plan.body.is_some() || r.plan.control.is_none() || r.plan.submit.is_some()) {
        return Err(StoreError::Invalid("control plan shape"));
    }
    match &p.ingress.submit {
        SubmitContract::OwnedAtAdmission if r.plan.submit.is_some()=>{},
        SubmitContract::SelfContainedInBody if r.plan.body.is_some() && r.plan.submit.is_none()=>{},
        SubmitContract::AwaitingAssociation { window_ms } if *window_ms>0 && p.ingress.intent==InputIntent::AgentText
            && r.plan.body.is_some() && r.plan.submit.is_none() && p.ingress.caller.is_some()=>{},
        SubmitContract::BoundToCompletedBody { body_request_id,body_lineage } if
            p.ingress.intent==InputIntent::AgentSubmit && !body_request_id.is_empty()
            && body_lineage.epochs.binding>0=>{},
        SubmitContract::NotApplicable if p.ingress.intent==InputIntent::AgentControl=>{},
        _=>return Err(StoreError::Invalid("submit contract does not match exact plan")),
    }
    if p.ingress.intent==InputIntent::AgentSubmit && !matches!(p.ingress.submit,SubmitContract::BoundToCompletedBody { .. }) {
        return Err(StoreError::Invalid("standalone submit lacks original body lineage"));
    }
    if let Some(s)=&p.selected { selected_shape(r,p,s)?; }
    Ok(())
}
fn selected_shape(r:&PendingRecord,p:&ProtocolData,s:&SelectedLineage)->Result<(),StoreError> {
    if s.catalog_id!=p.initial.catalog_id || s.adapter.is_empty() || s.config_digest.is_empty()
        || s.publication_ref.is_empty() || s.epochs.binding==0 || s.epochs.policy!=p.initial.policy_epoch
        || s.epochs.daemon_instance!=p.ingress.scope.daemon_instance
        || s.epochs.surface_instance!=p.ingress.scope.surface_instance
        || s.epochs.launch!=p.ingress.scope.launch || s.epochs.conversation!=p.ingress.scope.conversation
        || (s.class==ProofClass::FullRequired)!=s.weak_risk.is_none() {
        return Err(StoreError::Invalid("initial selected lineage shape"));
    }
    let _=r; // addressing remains the immutable common record, not a role re-resolution
    encode_limited(s,p.selected_reserve)?;
    Ok(())
}
fn protocol_registry_reference(record:&PendingRecord,registry:&RegistryImage)->Result<(),StoreError> {
    let RecordLineage::Protocol { data:p }=&record.lineage else { return Ok(()); };
    let registered=registry.registrations.get(&p.initial.registration_id).ok_or(StoreError::Invalid("initial registration reference"))?;
    if registered.catalog_id!=p.initial.catalog_id || registered.contract_key!=p.initial.contract_key
        || registered.scope!=record.admission_scope() { return Err(StoreError::Invalid("initial catalog reference")); }
    if let Some(selected)=&p.selected {
        let image=&registry.catalogs.get(&p.initial.catalog_id).ok_or(StoreError::Invalid("initial catalog absent"))?.image;
        let image:serde_json::Value=serde_json::from_str(image).map_err(|_|StoreError::Invalid("catalog JSON"))?;
        let entries=image["entries"].as_array().ok_or(StoreError::Invalid("catalog entries"))?;
        let entry=entries.iter().find(|e|e["policy"]["adapter"].as_str()==Some(selected.adapter.as_str()))
            .ok_or(StoreError::Invalid("selected adapter outside initial catalog"))?;
        if entry["policy"]["config_digest"].as_str()!=Some(selected.config_digest.as_str())
            || entry["policy"]["class"]!=serde_json::to_value(selected.class).map_err(|_|StoreError::Invalid("class codec"))?
            || entry["policy"]["weak_risk"]!=serde_json::to_value(selected.weak_risk).map_err(|_|StoreError::Invalid("risk codec"))? {
            return Err(StoreError::Invalid("selected contract differs from initial catalog"));
        }
    }
    Ok(())
}
fn selection_charge(record:&PendingRecord,l:Limits)->Result<usize,StoreError> {
    let RecordLineage::Protocol { data:p }=&record.lineage else { return Ok(0); };
    if p.selected_reserve>l.max_record_bytes { return Err(StoreError::Capacity("selected lineage reserve")); }
    let used=encode_limited(&p.selected,p.selected_reserve)?.len();
    let future=p.selected_reserve.checked_sub(used).ok_or(StoreError::Capacity("selected lineage bytes"))?;
    let revision_extra=u64::MAX.to_string().len().checked_sub(p.revision.to_string().len()).ok_or(StoreError::Invalid("revision width"))?;
    future.checked_add(revision_extra).ok_or(StoreError::Capacity("selected lineage reserve"))
}

impl PendingStore {
    /// A durable CurrentRequest owner only, not a writer AttemptLease. Every
    /// executable automatic producer must still use the managed writer protocol.
    pub fn accept_protocol<A:IngressAuthority>(&self,spec:NewProtocolSpec,
        registration:&Arc<RegisteredCatalog>,authority:A,bounds:ProtocolLimits)
        ->Result<Arc<CurrentRequest>,ProtocolError>
    {
        if spec.ingress.intent==InputIntent::AgentSubmit { return Err(ProtocolError::NeedsExistingRequest); }
        if bounds.max_selected_lineage_bytes<128 || bounds.max_selected_lineage_bytes>self.limits.max_record_bytes
            || spec.origin!=spec.ingress.origin { return Err(ProtocolError::Conflict); }
        // Capture the existing association policy at ORIGINAL receipt, never now.
        // Reopened historical windows are data only; no live deadline is rebuilt.
        let association_deadline=match &spec.ingress.submit {
            SubmitContract::AwaitingAssociation { window_ms }=>{
                if *window_ms!=crate::submit_assoc::LEGACY_ASSOCIATION_WINDOW_MS {
                    return Err(ProtocolError::Conflict);
                }
                Some(spec.received_at.checked_add(Duration::from_millis(*window_ms))
                    .ok_or(ProtocolError::Conflict)?)
            }
            _=>None,
        };
        // The real ingress factory runs OUTSIDE storage/native/input/queue locks.
        authority.validate_and_consume(&spec,registration)?;
        let view=spec.plan.exact_view();
        let payload=view.body.map_or(0,str::len).checked_add(view.control.map_or(0,<[u8]>::len))
            .ok_or(StoreError::Capacity("raw plan bytes"))?;
        if payload>self.limits.max_record_bytes { return Err(StoreError::Capacity("raw plan bytes").into()); }
        let exact=retained_plan(&spec.plan);
        let mut state=self.try_mutation()?;
        if state.uncertain { return Err(ProtocolError::Store(StoreError::UncertainCommit)); }
        self.validate_live_registration(&state,registration)?;
        let scope=registration.scope();let incoming=&spec.ingress.scope;
        if incoming.daemon_instance!=self.instance || incoming.daemon_instance!=scope.daemon_instance
            || incoming.surface_instance!=scope.surface_instance || incoming.launch!=scope.launch
            || spec.target.surface_id!=scope.surface_id { return Err(ProtocolError::Conflict); }
        let seq=state.next_request_seq;let next=seq.checked_add(1).ok_or(StoreError::Capacity("request sequence exhausted"))?;
        if seq==0 { return Err(ProtocolError::Conflict); }
        let id=format!("w{:x}-{seq}",self.instance);
        if state.rows.contains_key(&id) || state.current_requests.contains_key(&id) { return Err(ProtocolError::Conflict); }
        let protocol=ProtocolData { ingress:spec.ingress,initial:InitialCatalog {
            registration_id:registration.id().to_owned(),catalog_id:registration.record.catalog_id.clone(),
            contract_key:registration.record.contract_key.clone(),policy_epoch:scope.policy },
            selected:None,revision:1,selected_reserve:bounds.max_selected_lineage_bytes };
        let record=PendingRecord { request_id:id.clone(),registration_id:Some(registration.id().to_owned()),
            obligation_id:spec.obligation_id,source:spec.source,target:spec.target,
            lineage:RecordLineage::Protocol { data:protocol },original_body_digest:spec.original_body_digest,
            received_epoch_ms:spec.received_epoch_ms,plan:exact };
        validate_record(&record)?;protocol_registry_reference(&record,&state.registry)?;
        let row=Row { record,terminal:None,settled:None,attempts:None };checked_row_charge(&row,self.limits)?;
        let digest=row.record.original_body_digest.clone();let surface_id=row.record.target.surface_id;
        let mut rows=(*state.rows).clone();rows.insert(id.clone(),row);
        let mut registry=state.registry.clone();let ns=state.next_settle_seq;
        let bytes=prune_and_encode(&mut rows,next,ns,self.limits,None,&mut registry,self.registry_limits)?;
        self.commit(&mut state,rows,next,ns,registry,&bytes)?;

        // Post-commit runtime installation is also fail-closed on unwind. A
        // persistent row without a returned live owner remains protected data.
        let ambiguous=Arc::new(ReadSnapshot { rows:state.rows.clone(),loaded:state.loaded.clone(),
            uncertain:true,registry:state.registry.clone(),runtime_registrations:state.live_registrations.values()
                .map(|r|(r.id().to_owned(),Arc::downgrade(r))).collect() });
        let mut boundary=CommitBoundary { state:&mut state,published:&self.published,
            ambiguous:Some(ambiguous),identity:&self.identity };
        // REQUIRED root API; never substitute legacy Request::new. Managed writer
        // requests cannot enter a job until root's actual AttemptLease exists.
        let plan=Arc::new(spec.plan);
        let request=Request::new_managed_at(id.clone(),surface_id,spec.origin,digest,spec.received_at,&plan);
        let current=Arc::new(CurrentRequest { owner:Arc::downgrade(&self.identity),request,plan,
            initial_registration:registration.clone(),association_deadline });
        boundary.state.current_requests.insert(id,Arc::downgrade(&current));
        self.publish(boundary.state);boundary.ambiguous=None;
        Ok(current)
    }

    fn validate_current_locked<'a>(&self,state:&'a State,current:&Arc<CurrentRequest>,expected:u64)
        ->Result<(&'a Row,&'a ProtocolData),ProtocolError>
    {
        if state.uncertain { return Err(ProtocolError::Store(StoreError::UncertainCommit)); }
        let owner=current.owner.upgrade().ok_or(ProtocolError::Inactive)?;
        if !Arc::ptr_eq(&owner,&self.identity) || !owner.healthy.load(Ordering::Acquire)
            || !state.current_requests.get(current.id()).and_then(Weak::upgrade)
                .is_some_and(|original|Arc::ptr_eq(&original,current)) {
            return Err(ProtocolError::Inactive);
        }
        if state.loaded.contains(current.id()) { return Err(ProtocolError::RecoveredOnly); }
        let row=state.rows.get(current.id()).ok_or(ProtocolError::Missing)?;
        let RecordLineage::Protocol { data }=&row.record.lineage else { return Err(ProtocolError::Conflict); };
        if data.revision!=expected { return Err(ProtocolError::RevisionChanged); }
        if row.record.request_id!=current.request.id || row.record.target.surface_id!=current.request.surface_id
            || row.record.original_body_digest!=current.request.digest || !plan_matches(&row.record.plan,&current.plan)
            || data.ingress.origin!=current.request.origin || row.settled.is_some() {
            return Err(ProtocolError::Conflict);
        }
        Ok((row,data))
    }
    /// Checks runtime ownership plus exact DURABLE revision; it grants no attempt,
    /// proof or replacement authority and is not a recovered-row reconstruction.
    pub fn current_revision(&self,current:&Arc<CurrentRequest>,expected:u64)->Result<ProtocolData,ProtocolError> {
        let state=self.try_mutation()?;
        let (_,data)=self.validate_current_locked(&state,current,expected)?;
        Ok(data.clone())
    }

    /// INITIAL Agent selection only. Rebinding, process-specific control/Return
    /// transfer and shell LifecycleLaunch proof are deliberately separate future
    /// paths. They must not be smuggled through this initial-selection method.
    pub fn select_initial_lineage<A:InitialSelectionAuthority>(&self,
        current:&Arc<CurrentRequest>,expected:u64,mut edit:ZeroWriteEditLease,authority:A)->Result<u64,ProtocolError>
    {
        // The real root-owned reservation is held by value through this function.
        // Its verifier must check queue/Prepared ownership, not only running.
        edit.validate_reserved(&current.request,&current.plan)?;
        let (ingress,initial,prior)={
            let state=self.try_mutation()?;let (row,p)=self.validate_current_locked(&state,current,expected)?;
            if row.terminal.is_some() || p.ingress.target_kind!=TargetKind::Agent
                || p.ingress.intent==InputIntent::AgentSubmit || current.request.is_cancelled() {
                return Err(ProtocolError::Unsupported);
            }
            (p.ingress.clone(),p.initial.clone(),p.selected.clone())
        };
        // Actual controller authority validation stays OUTSIDE the store mutex.
        // A data-only selected lineage or archived JSON cannot implement it.
        let selected=authority.validate_and_consume(&current.request,&current.plan,&ingress,&initial,expected)?;
        edit.validate_held(&current.request,&current.plan)?;
        let mut state=self.try_mutation()?;
        let (row,p)=self.validate_current_locked(&state,current,expected)?;
        if row.terminal.is_some() || p.ingress!=ingress || p.initial!=initial || p.selected!=prior
            || current.request.is_cancelled() { return Err(ProtocolError::Conflict); }
        // Initial registration may have been revoked during authority collection.
        self.validate_live_registration(&state,&current.initial_registration)?;
        selected_shape(&row.record,p,&selected)?;
        if let Some(existing)=&p.selected {
            return if existing==&selected { Ok(p.revision) } else { Err(ProtocolError::Conflict) };
        }
        let revision=p.revision.checked_add(1).ok_or(StoreError::Capacity("request revision exhausted"))?;
        let mut rows=(*state.rows).clone();let updated=rows.get_mut(current.id()).expect("validated current row");
        let RecordLineage::Protocol { data }=&mut updated.record.lineage else { unreachable!() };
        data.selected=Some(selected);data.revision=revision;
        protocol_registry_reference(&updated.record,&state.registry)?;
        let mut registry=state.registry.clone();let nr=state.next_request_seq;let ns=state.next_settle_seq;
        let bytes=prune_and_encode(&mut rows,nr,ns,self.limits,None,&mut registry,self.registry_limits)?;
        // Only the existing Request.native exclusion is used. Its mutex is
        // released by begin_persisting before disk IO; no native query occurs.
        // Writer journaling must likewise call store methods AFTER releasing native.
        let stamp=edit.begin_persisting(&current.plan)?;
        let old_rows=state.rows.clone();
        match self.commit(&mut state,rows,nr,ns,registry,&bytes) {
            Ok(durability)=>{
                let receipt=DurableSelectionCommit { owner:Arc::downgrade(&self.identity),
                    current:current.clone(),stamp,before:expected,after:revision,durability };
                drop(state);
                // Actual crossmodule typed resolver, not caller-asserted success.
                // A cancellation/conflict leaves Persisting held for reconciliation.
                edit.resolve_selection(receipt,expected)?;
                // Data revision only: actual native/controller/input authority must
                // still be revalidated at handoff. The exact Plan never changed.
                Ok(revision)
            }
            Err(error)=>{
                // Private Disk/CommitBoundary proves the pre-install refusal. Both
                // authoritative store state AND exact original row Arc/revision
                // must remain unchanged before we release native exclusion.
                let not_written=matches!(&error,StoreError::Io { installed:false,.. })
                    && !state.uncertain && self.identity.healthy.load(Ordering::Acquire)
                    && Arc::ptr_eq(&old_rows,&state.rows)
                    && self.validate_current_locked(&state,current,expected).is_ok();
                if not_written {
                    let receipt=DurableSelectionNotWritten { owner:Arc::downgrade(&self.identity),
                        current:current.clone(),stamp,unchanged:expected };
                    drop(state);
                    edit.resolve_selection_not_written(receipt,expected)?;
                }
                // Installed/uncertain errors leave the lease Persisting on Drop.
                // This API never treats a generic IO error as evidence of rollback.
                Err(error.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("pending_aftercare_tests.rs");
    include!("pending_aftercare_batch2_tests.rs");
    use std::sync::atomic::{AtomicBool,AtomicU64,Ordering};
    use std::sync::mpsc;
    use std::time::Duration;
    mod ci_inputs { include!("c8_test_inputs.rs"); }
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name:&str)->Self {
            Self(ci_inputs::private_root("C8_PENDING_TEST_ROOT", "p")
                .unwrap_or_else(|error| panic!("C8 pending fixture {name} root provisioning failed: {error}")))
        }
    }
    impl Drop for Scratch { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
    fn limits()->Limits {
        Limits { max_active_records:8,max_record_bytes:8192,max_file_bytes:65536,
            max_terminal_bytes:2048,max_settlement_bytes:1024,max_recent_records:2,max_recent_bytes:20000 }
    }
    fn spec(label:&str)->NewSpec {
        NewSpec { obligation_id:format!("obligation-{label}"),
            source:SourceRef { kind:SourceKind::Direct,ids:vec![format!("source-{label}")] },
            target:Target { mode:TargetMode::Role,surface_id:7,role:Some("worker".into()) },
            epochs:EpochStamp { daemon_instance:11,surface_instance:12,launch:13,conversation:14,policy:15,binding:16 },
            policy_digest:"policy-sha".into(),original_body_digest:"body-sha".into(),received_epoch_ms:1790000000123,
            plan:ExactPlan { clear_first:true,clear_settle_ms:37,body:Some("private body\n한글\u{0}".into()),
                fenced:true,control:Some(vec![0,3,13,27,255]),submit:Some(ExactSubmit { delay_ms:421,min_gap_ms:159 }) } }
    }
    fn new_id(store:&PendingStore,label:&str)->String {
        match store.create_new(spec(label)).unwrap() {
            Admission::NewlyStored { permit,.. }=>permit.request_id().to_string(),other=>panic!("not new: {other:?}") }
    }
    fn terminal(id:&str,reason:RetiredReason)->TerminalRecord {
        TerminalRecord { reason,at_epoch_ms:500,evidence:serde_json::json!({"request_id":id,"fixture":"opaque private terminal"}) }
    }
    fn settlement(kind:SettlementKind,at:u64)->SettlementReceipt {
        SettlementReceipt { kind,at_epoch_ms:at,evidence:serde_json::json!({"fixture":"opaque private consumer receipt"}) }
    }
    fn finish(store:&PendingStore,id:&str,at:u64) {
        store.record_terminal(id,terminal(id,RetiredReason::WrittenSubmitted)).unwrap();
        store.mark_settled(id,settlement(SettlementKind::ProducerSettled,at)).unwrap();
    }
    fn write_private(path:&Path,bytes:&[u8]) {
        let mut o=OpenOptions::new();o.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; o.mode(0o600); }
        let mut f=o.open(path).unwrap();f.write_all(bytes).unwrap();f.sync_all().unwrap();
    }

    #[test]
    fn exact_plan_roundtrip_and_recovered_join_never_make_a_permit() {
        let scratch=Scratch::new("roundtrip");
        let record;
        {
            let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
            let id=new_id(&store,"one");
            record=store.inspect_exact_record(&id).unwrap();
            assert_eq!(record.plan,spec("one").plan);
            assert!(matches!(store.join_existing(&record),Ok(Admission::ExistingPending)));
            assert!(!format!("{:?}",store.inventory().unwrap()).contains("private body"));
        }
        let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        assert_eq!(store.inspect_exact_record(&record.request_id).unwrap(),record);
        assert!(matches!(store.join_existing(&record),Ok(Admission::RecoveredOnly)),"recovered work never grants an execution permit");
        assert!(store.inventory().unwrap()[0].recovered);
    }
    #[test]
    fn none_and_empty_exact_plan_fields_stay_distinct() {
        let scratch=Scratch::new("empty");let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let mut a=spec("none");a.plan.body=None;a.plan.control=None;a.plan.submit=None;
        let mut b=spec("empty");b.plan.body=Some(String::new());b.plan.control=Some(Vec::new());
        b.plan.submit=Some(ExactSubmit { delay_ms:0,min_gap_ms:0 });
        let expected_a=a.plan.clone();let expected_b=b.plan.clone();
        let id_a=match store.create_new(a).unwrap() { Admission::NewlyStored { permit,.. }=>permit.request_id().to_owned(),_=>panic!() };
        let id_b=match store.create_new(b).unwrap() { Admission::NewlyStored { permit,.. }=>permit.request_id().to_owned(),_=>panic!() };
        assert_eq!(store.inspect_exact_record(&id_a).unwrap().plan,expected_a);
        assert_eq!(store.inspect_exact_record(&id_b).unwrap().plan,expected_b);
        assert_ne!(expected_a,expected_b);
    }
    #[test]
    fn lookup_conflicts_or_misses_never_create_an_entry() {
        let scratch=Scratch::new("lookup");let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let id=new_id(&store,"one");let record=store.inspect_exact_record(&id).unwrap();
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        let mut changed=record.clone();changed.plan.clear_settle_ms+=1;
        assert!(matches!(store.join_existing(&changed),Err(StoreError::Conflict)));
        changed.request_id="wb-999999".into();
        assert!(matches!(store.join_existing(&changed),Ok(Admission::StatusUnavailable)),"missing request lookup never creates executable work");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert_eq!(store.inventory().unwrap().len(),1);
    }
    #[test]
    fn many_successes_do_not_fill_active_capacity() {
        let scratch=Scratch::new("healthy");let l=Limits { max_active_records:2,..limits() };
        let store=PendingStore::open(&scratch.0,11,l).unwrap();let mut first=None;
        for n in 0..20 {
            let id=new_id(&store,&format!("n{n}"));
            if first.is_none() { first=Some(store.inspect_exact_record(&id).unwrap()); }
            finish(&store,&id,n);
            assert!(store.inspect_terminal(&id).unwrap().is_some(),"new closure must be installed");
            assert!(store.inventory().unwrap().len()<=l.max_recent_records);
            assert!(fs::metadata(scratch.0.join(DATA)).unwrap().len()<=l.max_file_bytes as u64);
        }
        let first=first.unwrap();
        assert!(matches!(store.join_existing(&first),Ok(Admission::StatusUnavailable)));
        assert!(matches!(store.inspect_exact_record(&first.request_id),Err(StoreError::Missing)));
    }
    #[test]
    fn successful_but_unsettled_receipts_are_protected() {
        let scratch=Scratch::new("unsettled");let store=PendingStore::open(&scratch.0,11,
            Limits { max_active_records:2,..limits() }).unwrap();
        let a=new_id(&store,"a");let b=new_id(&store,"b");
        store.record_terminal(&a,terminal(&a,RetiredReason::WrittenSubmitted)).unwrap();
        store.record_terminal(&b,terminal(&b,RetiredReason::WrittenSubmitted)).unwrap();
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        assert!(matches!(store.create_new(spec("c")),Err(StoreError::Capacity("active records"))),"successful but unsettled work still owns protected capacity");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        store.mark_settled(&a,settlement(SettlementKind::ProducerSettled,1)).unwrap();
        new_id(&store,"c");
        assert!(store.inspect_exact_record(&b).is_ok());
    }
    #[test]
    fn partial_and_refused_work_cannot_use_plain_producer_settled() {
        let scratch=Scratch::new("partial");let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        for (n,reason) in [RetiredReason::PartialUnknown,RetiredReason::RefusedNoWrite,RetiredReason::CancelledBeforeWrite].into_iter().enumerate() {
            let id=new_id(&store,&format!("p{n}"));store.record_terminal(&id,terminal(&id,reason)).unwrap();
            assert!(matches!(store.mark_settled(&id,settlement(SettlementKind::ProducerSettled,1)),
                Err(StoreError::Invalid("explicit work disposition required"))));
            assert!(matches!(store.inventory().unwrap().iter().find(|r|r.request_id==id).unwrap().disposition,
                Disposition::TerminalProtected { .. }));
        }
    }
    #[test]
    fn explicit_consumer_disposition_is_separate_from_terminal() {
        let scratch=Scratch::new("disposition");let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let id=new_id(&store,"unknown");
        assert!(matches!(store.mark_settled(&id,settlement(SettlementKind::Superseded,9)),
            Err(StoreError::Invalid("no durable terminal evidence"))));
        let t=terminal(&id,RetiredReason::PartialUnknown);store.record_terminal(&id,t.clone()).unwrap();
        assert!(matches!(store.record_terminal(&id,t.clone()),Ok(PersistResult::Identical)));
        let close=settlement(SettlementKind::Superseded,9);store.mark_settled(&id,close.clone()).unwrap();
        assert!(matches!(store.mark_settled(&id,close),Ok(PersistResult::Identical)));
        assert!(matches!(store.mark_settled(&id,settlement(SettlementKind::AlreadyCompleted,10)),Err(StoreError::SettlementConflict)));
        assert_eq!(store.inspect_terminal(&id).unwrap(),Some(t));
        let printed=format!("{:?} {:?} {:?}",store.inventory().unwrap(),store.inspect_terminal(&id).unwrap(),store.inspect_settlement(&id).unwrap());
        assert!(!printed.contains("opaque private") && !printed.contains("private body"));
    }
    #[test]
    fn persisted_sequence_survives_eviction_and_same_instance_reopen() {
        let scratch=Scratch::new("sequence");let maximum;
        {
            let store=PendingStore::open(&scratch.0,11,Limits { max_recent_records:1,..limits() }).unwrap();
            let mut last=String::new();for n in 0..5 { last=new_id(&store,&format!("n{n}"));finish(&store,&last,n); }
            maximum=canonical_sequence(&last).unwrap();
        }
        let store=PendingStore::open(&scratch.0,11,Limits { max_recent_records:1,..limits() }).unwrap();
        let next=new_id(&store,"later");assert!(canonical_sequence(&next).unwrap()>maximum);
    }
    #[test]
    fn eviction_uses_commit_order_not_wall_time() {
        let scratch=Scratch::new("clock");let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let mut ids=Vec::new();for at in [900,500,1] { let id=new_id(&store,"same");finish(&store,&id,at);ids.push(id); }
        assert!(matches!(store.inspect_exact_record(&ids[0]),Err(StoreError::Missing)));
        assert!(store.inspect_exact_record(&ids[2]).is_ok());
    }
    #[test]
    fn file_pressure_prunes_only_settled_history() {
        let scratch=Scratch::new("bytecap");let base=limits();
        let row=Row { attempts:None, record:spec("same").into_record("wb-1".into()),terminal:None,settled:None };
        let charged=checked_row_charge(&row,base).unwrap();
        let header=serde_json::to_vec(&Snapshot { schema:SCHEMA,next_request_seq:u64::MAX,next_settle_seq:u64::MAX,rows:vec![],registry:RegistryImage { next_registration_seq:u64::MAX,..RegistryImage::default() } }).unwrap().len();
        let l=Limits { max_record_bytes:charged,max_file_bytes:charged+header,
            max_recent_records:8,max_recent_bytes:charged+1,..base };
        let store=PendingStore::open(&scratch.0,11,l).unwrap();
        let a=new_id(&store,"same");finish(&store,&a,1);
        let b=new_id(&store,"same");
        assert!(store.inspect_exact_record(&b).is_ok(),
            "new admission must retain its protected record under byte pressure");
        assert!(matches!(store.inspect_exact_record(&a),Err(StoreError::Missing)));
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        assert!(matches!(store.create_new(spec("same")),Err(StoreError::Capacity(_))),"file pressure cannot evict protected work");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert!(store.inspect_exact_record(&b).is_ok());
    }
    #[test]
    fn terminal_and_settlement_headroom_is_reserved_before_writing() {
        let scratch=Scratch::new("reserve");let l=limits();let store=PendingStore::open(&scratch.0,11,l).unwrap();
        let id=new_id(&store,"edge");
        let mut t=TerminalRecord { reason:RetiredReason::WrittenSubmitted,at_epoch_ms:u64::MAX,evidence:serde_json::json!({"x":""}) };
        let base=serde_json::to_vec(&t).unwrap().len();t.evidence["x"]=serde_json::Value::String("x".repeat(l.max_terminal_bytes-base));
        assert_eq!(serde_json::to_vec(&t).unwrap().len(),l.max_terminal_bytes);
        store.record_terminal(&id,t).unwrap();
        let mut receipt=SettlementReceipt { kind:SettlementKind::ProducerSettled,at_epoch_ms:u64::MAX,evidence:serde_json::json!({"x":""}) };
        let base=serde_json::to_vec(&Settled { order:u64::MAX,receipt:receipt.clone() }).unwrap().len();
        receipt.evidence["x"]=serde_json::Value::String("x".repeat(l.max_settlement_bytes-base));
        assert_eq!(serde_json::to_vec(&Settled { order:u64::MAX,receipt:receipt.clone() }).unwrap().len(),l.max_settlement_bytes);
        store.mark_settled(&id,receipt).unwrap();
    }
    #[test]
    fn oversize_opaque_evidence_is_refused_without_mutation() {
        let scratch=Scratch::new("opaque-cap");let l=limits();let store=PendingStore::open(&scratch.0,11,l).unwrap();
        let id=new_id(&store,"x");let before=fs::read(scratch.0.join(DATA)).unwrap();
        let t=TerminalRecord { reason:RetiredReason::WrittenSubmitted,at_epoch_ms:1,evidence:serde_json::json!({"x":"x".repeat(l.max_terminal_bytes)}) };
        assert!(matches!(store.record_terminal(&id,t),Err(StoreError::Capacity(_))));
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert!(store.inspect_terminal(&id).unwrap().is_none());
    }
    struct SwitchDisk { fail:AtomicBool }
    impl Disk for SwitchDisk {
        fn replace(&self,root:&Path,bytes:&[u8])->Result<Durability,CommitFailure> {
            if self.fail.load(Ordering::SeqCst) { return Err(CommitFailure { stage:"fixture_before",installed:false,uncertain:false,message:"injected".into() }); }
            RealDisk.replace(root,bytes)
        }
    }
    #[test]
    fn failed_pruning_commit_keeps_old_rows_and_allocation_counter() {
        let scratch=Scratch::new("prune-fail");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
        let l=Limits { max_recent_records:1,..limits() };
        let store=PendingStore::open_with(&scratch.0,11,l,disk.clone()).unwrap();
        let a=new_id(&store,"a");finish(&store,&a,1);
        let b=new_id(&store,"b");store.record_terminal(&b,terminal(&b,RetiredReason::WrittenSubmitted)).unwrap();
        let before=fs::read(scratch.0.join(DATA)).unwrap();disk.fail.store(true,Ordering::SeqCst);
        assert!(matches!(store.mark_settled(&b,settlement(SettlementKind::ProducerSettled,2)),Err(StoreError::Io { installed:false,.. })));
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert!(store.inspect_exact_record(&a).is_ok(),"candidate pruning must not mutate committed map");
        assert!(store.inspect_settlement(&b).unwrap().is_none());
        assert!(matches!(store.create_new(spec("failed")),Err(StoreError::Io { installed:false,.. })),"failed durable admission grants no execution permit");
        disk.fail.store(false,Ordering::SeqCst);
        let c=new_id(&store,"c");assert_eq!(canonical_sequence(&c),Some(3));
        store.mark_settled(&b,settlement(SettlementKind::ProducerSettled,2)).unwrap();
        assert!(matches!(store.inspect_exact_record(&a),Err(StoreError::Missing)));
    }
    struct FailAfter;
    impl Disk for FailAfter {
        fn replace(&self,root:&Path,bytes:&[u8])->Result<Durability,CommitFailure> {
            RealDisk.replace(root,bytes)?;
            // Simulated caller-visible uncertainty, NOT a native fsync-fault claim.
            Err(CommitFailure { stage:"fixture_after",installed:true,uncertain:true,message:"injected".into() })
        }
    }
    #[test]
    fn installed_uncertainty_latches_and_reopens_without_execution() {
        let scratch=Scratch::new("uncertain");
        {
            let store=PendingStore::open_with(&scratch.0,11,limits(),Arc::new(FailAfter)).unwrap();
            assert!(matches!(store.create_new(spec("one")),Err(StoreError::Io { installed:true,.. })));
            assert!(matches!(store.inventory(),Err(StoreError::UncertainCommit)));
            assert!(matches!(store.create_new(spec("two")),Err(StoreError::UncertainCommit)));
        }
        let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let old=store.inspect_exact_record("wb-1").unwrap();
        assert!(matches!(store.join_existing(&old),Ok(Admission::RecoveredOnly)));
        assert_eq!(new_id(&store,"new"),"wb-2");
    }
    #[test]
    fn loaded_id_bookkeeping_is_reclaimed_with_the_row() {
        let scratch=Scratch::new("loaded");let l=Limits { max_recent_records:1,..limits() };let a;
        { let store=PendingStore::open(&scratch.0,11,l).unwrap();a=new_id(&store,"a"); }
        let store=PendingStore::open(&scratch.0,11,l).unwrap();
        finish(&store,&a,1);let b=new_id(&store,"b");finish(&store,&b,2);
        let state=store.snapshot();
        assert!(!state.rows.contains_key(&a));
        assert!(!state.loaded.contains(&a),"separate loaded-ID set must not become eternal tombstones");
        assert!(state.loaded.iter().all(|id|state.rows.contains_key(id)));
    }
    #[test]
    fn unknown_schema_is_rejected_without_overwrite() {
        let scratch=Scratch::new("unknown-schema");
        let bytes=serde_json::to_vec(&serde_json::json!({"schema":1,"rows":[]})).unwrap();
        write_private(&scratch.0.join(DATA),&bytes);
        assert!(PendingStore::open(&scratch.0,11,limits()).is_err());
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),bytes);
    }
    #[test]
    fn exhausted_request_counter_refuses_without_wrap_or_write() {
        let scratch=Scratch::new("exhausted");let snap=Snapshot { schema:SCHEMA,next_request_seq:u64::MAX,next_settle_seq:1,rows:vec![],registry:RegistryImage::default() };
        let before=serde_json::to_vec(&snap).unwrap();write_private(&scratch.0.join(DATA),&before);
        let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        assert!(matches!(store.create_new(spec("x")),Err(StoreError::Capacity("request sequence exhausted"))));
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
    }
    #[test]
    fn duplicate_id_and_bad_counter_are_rejected_without_overwrite() {
        for mode in ["duplicate","counter"] {
            let scratch=Scratch::new(mode);let row=Row { attempts:None, record:spec("x").into_record("wb-3".into()),terminal:None,settled:None };
            let rows=if mode=="duplicate" { vec![row.clone(),row] } else { vec![row] };
            let next=if mode=="duplicate" { 4 } else { 3 };
            let bytes=serde_json::to_vec(&Snapshot { schema:SCHEMA,next_request_seq:next,next_settle_seq:1,rows,registry:RegistryImage::default() }).unwrap();
            write_private(&scratch.0.join(DATA),&bytes);
            assert!(PendingStore::open(&scratch.0,11,limits()).is_err());
            assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),bytes);
        }
    }
    #[test]
    fn corrupt_snapshot_is_not_overwritten_and_one_store_owns_directory() {
        let scratch=Scratch::new("corrupt");write_private(&scratch.0.join(DATA),b"{broken");
        assert!(PendingStore::open(&scratch.0,11,limits()).is_err());
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),b"{broken");
        let other=Scratch::new("lock");let first=PendingStore::open(&other.0,11,limits()).unwrap();
        assert!(matches!(PendingStore::open(&other.0,11,limits()),Err(StoreError::Locked)));
        drop(first);assert!(PendingStore::open(&other.0,11,limits()).is_ok());
    }
    #[test]
    fn foreign_staging_latches_uncertainty_without_deleting_it() {
        let scratch=Scratch::new("foreign-temp");let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let text=b"not created by this admission";write_private(&scratch.0.join(TEMP),text);
        assert!(matches!(store.create_new(spec("x")),Err(StoreError::Io { installed:false,.. })));
        assert_eq!(fs::read(scratch.0.join(TEMP)).unwrap(),text);
        assert!(matches!(store.inventory(),Err(StoreError::UncertainCommit)));
    }
    struct BlockingDisk { entered:mpsc::Sender<()>,release:Mutex<mpsc::Receiver<()>>,block_once:AtomicBool }
    impl Disk for BlockingDisk {
        fn replace(&self,root:&Path,bytes:&[u8])->Result<Durability,CommitFailure> {
            if self.block_once.swap(false,Ordering::SeqCst) {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv_timeout(Duration::from_secs(10)).expect("test releases disk");
            }
            RealDisk.replace(root,bytes)
        }
    }
    #[test]
    fn status_and_join_do_not_wait_for_persistence_and_mutations_report_busy() {
        let scratch=Scratch::new("nonblocking");let original;
        {
            let store=PendingStore::open(&scratch.0,11,limits()).unwrap();
            let id=new_id(&store,"existing");original=store.inspect_exact_record(&id).unwrap();
            store.record_terminal(&id,terminal(&id,RetiredReason::WrittenSubmitted)).unwrap();
        }
        let (entered,reached)=mpsc::channel();let (release,wait)=mpsc::channel();
        let store=Arc::new(PendingStore::open_with(&scratch.0,11,limits(),Arc::new(BlockingDisk { entered,release:Mutex::new(wait),block_once:AtomicBool::new(true) })).unwrap());
        let writer_store=store.clone();let writer=std::thread::spawn(move||writer_store.create_new(spec("inflight")));
        reached.recv_timeout(Duration::from_secs(10)).expect("writer entered disk");
        let (done,answer)=mpsc::channel();let observer=store.clone();
        let read=std::thread::spawn(move||{
            let count=observer.inventory().unwrap().len();
            let join=matches!(observer.join_existing(&original),Ok(Admission::RecoveredOnly));
            let new_busy=matches!(observer.create_new(spec("contender")),Err(StoreError::Busy));
            let terminal_busy=matches!(observer.record_terminal(&original.request_id,terminal(&original.request_id,RetiredReason::WrittenSubmitted)),Err(StoreError::Busy));
            let settle_busy=matches!(observer.mark_settled(&original.request_id,settlement(SettlementKind::ProducerSettled,1)),Err(StoreError::Busy));
            done.send((count,join,new_busy,terminal_busy,settle_busy)).unwrap();
        });
        let observed=answer.recv_timeout(Duration::from_secs(2));
        // Release/join BEFORE asserting so a RED does not strand its fixture writer.
        release.send(()).unwrap();let written=writer.join().unwrap();read.join().unwrap();
        assert!(observed.is_ok(),"read or contending mutation waited on fsync");
        assert_eq!(observed.unwrap(),(1,true,true,true,true));
        assert!(matches!(written,Ok(Admission::NewlyStored { .. })));
        assert_eq!(store.inventory().unwrap().len(),2);
    }
    #[test]
    fn status_miss_does_not_reallocate_or_replay() {
        let scratch=Scratch::new("status");let store=PendingStore::open(&scratch.0,11,
            Limits { max_recent_records:1,..limits() }).unwrap();
        let first=new_id(&store,"first");finish(&store,&first,1);
        let second=new_id(&store,"second");finish(&store,&second,2);
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        assert!(store.status(&first).unwrap().is_none());
        assert!(store.status("not-issued").unwrap().is_none());
        assert!(matches!(store.status(&second).unwrap().unwrap().disposition,Disposition::SettledRecent { .. }));
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert_eq!(new_id(&store,"third"),"wb-3");
    }

    struct PanicOnceAfterReplace(AtomicBool);
    impl Disk for PanicOnceAfterReplace {
        fn replace(&self,root:&Path,bytes:&[u8])->Result<Durability,CommitFailure> {
            let level=RealDisk.replace(root,bytes)?;
            if self.0.swap(false,Ordering::SeqCst) { panic!("fixture panic after durable replacement"); }
            Ok(level)
        }
    }
    #[test]
    fn c8store_commit_unwind_quarantines_memory_and_preserves_installed_row() {
        let scratch=Scratch::new("commit-unwind");
        let (read_refused,mutation_refused,unchanged);
        {
            let store=PendingStore::open_with(&scratch.0,11,limits(),
                Arc::new(PanicOnceAfterReplace(AtomicBool::new(true)))).unwrap();
            let thrown=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||store.create_new(spec("first"))));
            assert!(thrown.is_err(),"premise: real replacement completed before injected unwind");
            let before=fs::read(scratch.0.join(DATA)).unwrap();
            read_refused=matches!(store.inventory(),Err(StoreError::UncertainCommit));
            mutation_refused=matches!(store.create_new(spec("second")),Err(StoreError::UncertainCommit));
            unchanged=fs::read(scratch.0.join(DATA)).unwrap()==before;
        }
        let reopened=PendingStore::open(&scratch.0,11,limits()).unwrap();
        let recovered=reopened.inspect_exact_record("wb-1").unwrap();
        let original_preserved=recovered.obligation_id=="obligation-first";
        let recovery_only=matches!(reopened.join_existing(&recovered),Ok(Admission::RecoveredOnly));
        let next=new_id(&reopened,"after-reopen");
        assert!(read_refused && mutation_refused && unchanged && original_preserved && recovery_only,
            "post-replace unwind must quarantine stale memory and preserve installed work: read={read_refused} mutation={mutation_refused} disk={unchanged} original={original_preserved} recovery={recovery_only}");
        assert_eq!(next,"wb-2","reopen continues the durable request high-water mark");
    }
    #[test]
    fn c8store_load_enforces_live_settlement_compatibility_without_overwrite() {
        let mut results=Vec::new();
        for reason in [RetiredReason::RefusedNoWrite,RetiredReason::CancelledBeforeWrite,
            RetiredReason::PartialUnknown,RetiredReason::ExplicitUndelivered] {
            let mut observed=(false,true,false);
            for valid in [false,true] {
                let scratch=Scratch::new("load-settlement");
                let kind=if valid { SettlementKind::ExplicitUndelivered } else { SettlementKind::ProducerSettled };
                let row=Row { attempts:None, record:spec("loaded").into_record("wb-1".into()),
                    terminal:Some(terminal("wb-1",reason)),
                    settled:Some(Settled { order:1,receipt:settlement(kind,501) }) };
                let bytes=serde_json::to_vec(&Snapshot { schema:SCHEMA,next_request_seq:2,next_settle_seq:2,rows:vec![row],registry:RegistryImage::default() }).unwrap();
                write_private(&scratch.0.join(DATA),&bytes);
                let opened=PendingStore::open(&scratch.0,11,limits());
                let accepted=opened.is_ok();
                if valid { observed.2=accepted; } else { observed.0=accepted; }
                observed.1 &= fs::read(scratch.0.join(DATA)).unwrap()==bytes;
            }
            results.push((reason,observed));
        }
        assert!(results.iter().all(|(_,v)|!v.0 && v.1 && v.2),
            "load must enforce the same settlement pairs as live mutation without changing disk: {results:?}");
    }

    #[test]
    fn c8store_high_water_survives_eviction_of_the_highest_issued_id() {
        let scratch=Scratch::new("highest-id-evicted");
        let l=Limits { max_recent_records:1,..limits() };
        {
            let store=PendingStore::open(&scratch.0,11,l).unwrap();
            let older=new_id(&store,"older");let highest=new_id(&store,"highest");
            assert_eq!((&older,&highest),(&"wb-1".to_string(),&"wb-2".to_string()));
            finish(&store,&highest,1);finish(&store,&older,2);
            assert!(store.inspect_exact_record(&highest).is_err(),"premise: highest ID is actually evicted");
            assert_eq!(store.inventory().unwrap().len(),1);
        }
        let reopened=PendingStore::open(&scratch.0,11,l).unwrap();
        assert_eq!(new_id(&reopened,"next"),"wb-3",
            "durable high-water cannot be reconstructed from only retained IDs");
    }
    #[test]
    fn c8store_admission_reserves_future_receipts_under_file_pressure() {
        let scratch=Scratch::new("future-receipts");
        // Fixed independent budget: one maximal receipt fits; two reserved rows do not.
        let l=Limits { max_record_bytes:4096,max_file_bytes:6000,..limits() };
        let store=PendingStore::open(&scratch.0,11,l).unwrap();
        let id=new_id(&store,"one");
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        assert!(matches!(store.create_new(spec("second")),Err(StoreError::Capacity(_))),
            "admission must reserve terminal and settlement bytes before granting a permit");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        let mut t=terminal(&id,RetiredReason::WrittenSubmitted);
        t.evidence=serde_json::json!({"x":""});t.at_epoch_ms=u64::MAX;
        let base=serde_json::to_vec(&t).unwrap().len();
        t.evidence["x"]=serde_json::Value::String("x".repeat(l.max_terminal_bytes-base));
        assert_eq!(serde_json::to_vec(&t).unwrap().len(),l.max_terminal_bytes);
        store.record_terminal(&id,t).unwrap();
        let mut receipt=settlement(SettlementKind::ProducerSettled,u64::MAX);
        receipt.evidence=serde_json::json!({"x":""});
        let base=serde_json::to_vec(&Settled { order:u64::MAX,receipt:receipt.clone() }).unwrap().len();
        receipt.evidence["x"]=serde_json::Value::String("x".repeat(l.max_settlement_bytes-base));
        assert_eq!(serde_json::to_vec(&Settled { order:u64::MAX,receipt:receipt.clone() }).unwrap().len(),l.max_settlement_bytes);
        store.mark_settled(&id,receipt).unwrap();
        assert!(fs::metadata(scratch.0.join(DATA)).unwrap().len()<=l.max_file_bytes as u64);
    }


    fn registry_limits()->RegistryLimits { RegistryLimits { max_catalogs:4,max_registrations:4,max_catalog_bytes:16000,max_declarations:8 } }
    fn registry_scope()->RegistrationScope { RegistrationScope { daemon_instance:11,surface_id:7,surface_instance:12,launch:13,policy:15 } }
    fn registry_catalog(epoch:u64,origin:crate::receiver::RegistrationOrigin,label:&str)->Arc<crate::policy_loader::Catalog> {
        let disk=serde_json::json!({"_schema":3,"custom":{"cmd":"custom-agent","env":{"VALUE":label},"prompt_marker":">"}});
        Arc::new(crate::policy_loader::load_catalog(Some(&disk),&serde_json::json!({}),epoch,origin,true).unwrap())
    }
    fn fresh_registry_catalog(label:&str)->Arc<crate::policy_loader::Catalog> {
        registry_catalog(15,crate::receiver::RegistrationOrigin::ClaimRole,label)
    }
    fn registered_id(store:&PendingStore,cap:&Arc<RegisteredCatalog>,label:&str)->String {
        match store.create_registered(spec(label),cap).unwrap() {
            Admission::NewlyStored { permit,.. }=>permit.request_id().to_owned(),other=>panic!("not fresh: {other:?}") }
    }
    struct CountingRegistryDisk(AtomicU64);
    impl Disk for CountingRegistryDisk {
        fn replace(&self,root:&Path,bytes:&[u8])->Result<Durability,CommitFailure> {
            self.0.fetch_add(1,Ordering::SeqCst);RealDisk.replace(root,bytes)
        }
    }

    #[test]
    fn c8registry_persists_before_use_and_reopen_is_data_only() {
        let scratch=Scratch::new("registry-recovery");let cap;let old_id;
        {
            let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
            cap=store.register_catalog(registry_scope(),fresh_registry_catalog("SYNTHETIC_PRIVATE_REGISTRY_VALUE")).unwrap();
            old_id=cap.id().to_owned();
            let disk:Snapshot=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
            let registration=&disk.registry.registrations[cap.id()];
            assert_eq!(registration.scope,registry_scope());
            assert!(disk.registry.catalogs[&registration.catalog_id].image.contains("SYNTHETIC_PRIVATE_REGISTRY_VALUE"),
                "registration capability returned before its exact private catalog was persisted");
            assert!(store.inventory().unwrap().is_empty(),"registration must not fabricate a writer request");
            assert!(cap.check_live().is_ok());
            assert!(store.registrations().unwrap()[0].usable_in_this_run);
            assert!(!format!("{:?}",store.registrations().unwrap()).contains("SYNTHETIC_PRIVATE_REGISTRY_VALUE"));
        }
        assert!(cap.check_live().is_err(),"registration authority outlived its store");
        let reopened=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
        let historical=reopened.registrations().unwrap();
        assert_eq!(historical[0].id,old_id);assert!(!historical[0].closed);
        assert!(!historical[0].usable_in_this_run,"recovery data became current registration authority");
        assert!(reopened.create_registered(spec("old"),&cap).is_err());
        let fresh=reopened.register_catalog(registry_scope(),fresh_registry_catalog("SYNTHETIC_PRIVATE_REGISTRY_VALUE")).unwrap();
        assert_ne!(fresh.id(),old_id,"registration high-water was lost on restart");
        assert!(fresh.check_live().is_ok());
        assert!(reopened.registrations().unwrap().iter().find(|r|r.id==fresh.id()).unwrap().usable_in_this_run);
    }

    #[test]
    fn c8registry_same_contract_is_a_persistence_free_noop() {
        let scratch=Scratch::new("registry-noop");let disk=Arc::new(CountingRegistryDisk(AtomicU64::new(0)));
        let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
        let first=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        let again=store.register_catalog(registry_scope(),registry_catalog(99,crate::receiver::RegistrationOrigin::Reclaim,"one")).unwrap();
        assert!(Arc::ptr_eq(&first,&again),"same contract must retain the original runtime capability");
        assert_eq!(again.record.origin,crate::receiver::RegistrationOrigin::ClaimRole);
        assert_eq!(again.scope().policy,15);
        assert_eq!(disk.0.load(Ordering::SeqCst),1,"metadata re-registration must not rewrite the policy archive");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert!(matches!(store.register_catalog(registry_scope(),fresh_registry_catalog("changed")),Err(StoreError::Conflict)),
            "a changed configuration silently replaced the existing policy scope");
        assert_eq!(disk.0.load(Ordering::SeqCst),1);
    }

    #[test]
    fn c8registry_request_reference_protects_closed_catalog_until_safe_reclamation() {
        let scratch=Scratch::new("registry-retention");let mut r=registry_limits();r.max_catalogs=1;r.max_registrations=1;
        let store=PendingStore::open_with_registry(&scratch.0,11,limits(),r).unwrap();
        let first=store.register_catalog(registry_scope(),fresh_registry_catalog("old")).unwrap();
        let id=registered_id(&store,&first,"protected");
        assert_eq!(store.inspect_exact_record(&id).unwrap().registration_id.as_deref(),Some(first.id()));
        store.close_registration(&first).unwrap();
        assert!(first.check_live().is_err());
        let mut next_scope=registry_scope();next_scope.policy=16;
        let next_catalog=||registry_catalog(16,crate::receiver::RegistrationOrigin::ClaimRole,"new");
        let before=fs::read(scratch.0.join(DATA)).unwrap();
        assert!(matches!(store.register_catalog(next_scope,next_catalog()),Err(StoreError::Capacity(_))),
            "a pending request's closed catalog was silently evicted");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
        assert!(store.inspect_private_catalog(&first.record.catalog_id).is_ok());
        finish(&store,&id,1);
        let next=store.register_catalog(next_scope,next_catalog()).unwrap();
        assert_ne!(next.id(),first.id());
        assert!(store.status(&id).unwrap().is_none(),"settled history was not reclaimed to make registry capacity");
        assert!(matches!(store.inspect_private_catalog(&first.record.catalog_id),Err(StoreError::Missing)));
        assert_eq!(store.registrations().unwrap().len(),1);
    }

    #[test]
    fn c8registry_foreign_or_moved_scope_cannot_admit_a_registered_request() {
        let a=Scratch::new("registry-owner-a");let b=Scratch::new("registry-owner-b");
        let first=PendingStore::open_with_registry(&a.0,11,limits(),registry_limits()).unwrap();
        let second=PendingStore::open_with_registry(&b.0,11,limits(),registry_limits()).unwrap();
        let cap=first.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        let twin=second.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        assert_eq!(cap.record,twin.record,"foreign-cap test requires equal persisted data in distinct stores");
        assert!(matches!(second.close_registration(&cap),Err(StoreError::Conflict)));
        assert!(cap.check_live().is_ok()&&twin.check_live().is_ok(),"foreign close revoked a registration before validating its owner");
        assert!(matches!(second.create_registered(spec("foreign"),&cap),Err(StoreError::Conflict)),"another store accepted a foreign capability");
        let before=fs::read(a.0.join(DATA)).unwrap();
        for axis in 0..5 {
            let mut moved=spec("moved");match axis {
                0=>moved.epochs.daemon_instance+=1,1=>moved.epochs.surface_instance+=1,
                2=>moved.epochs.launch+=1,3=>moved.epochs.policy+=1,4=>moved.target.surface_id+=1,_=>unreachable!() }
            assert!(first.create_registered(moved,&cap).is_err(),"scope axis {axis} ignored");
        }
        assert_eq!(fs::read(a.0.join(DATA)).unwrap(),before);
        let id=registered_id(&first,&cap,"valid");assert_eq!(id,"wb-1","rejected requests consumed the request high-water");
    }

    #[test]
    fn c8registry_failed_registration_grants_no_capability_or_counter_advance() {
        let scratch=Scratch::new("registry-failed");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(true) });
        let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
        assert!(matches!(store.register_catalog(registry_scope(),fresh_registry_catalog("one")),Err(StoreError::Io { .. })),
            "failed persistence returned a registration capability");
        assert!(store.registrations().unwrap().is_empty());
        disk.fail.store(false,Ordering::Release);
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        assert_eq!(cap.id(),"rb-1");assert!(cap.check_live().is_ok());
    }

    #[test]
    fn c8registry_failed_close_is_locally_revoked_and_retriable() {
        let scratch=Scratch::new("registry-close");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
        let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        disk.fail.store(true,Ordering::Release);
        assert!(matches!(store.close_registration(&cap),Err(StoreError::Io { .. })));
        assert!(cap.check_live().is_err(),"failed close revived local registration authority");
        let rows=store.registrations().unwrap();assert!(!rows[0].closed);
        assert!(!rows[0].usable_in_this_run,"status copied the stored open flag instead of reading revoked authority");
        assert!(store.create_registered(spec("after-close"),&cap).is_err());
        disk.fail.store(false,Ordering::Release);store.close_registration(&cap).unwrap();
        assert!(store.registrations().unwrap().is_empty());
        assert!(matches!(store.close_registration(&cap),Ok(PersistResult::Identical)));
    }

    #[test]
    fn c8registry_uncertain_commit_revokes_preexisting_current_capabilities() {
        let scratch=Scratch::new("registry-uncertain");let disk=Arc::new(PanicOnceAfterReplace(AtomicBool::new(false)));
        let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        disk.0.store(true,Ordering::Release);
        let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||store.create_registered(spec("panic"),&cap)));
        assert!(outcome.is_err());
        assert!(matches!(cap.check_live(),Err(StoreError::UncertainCommit)),"published registry capability ignored commit uncertainty");
        assert!(matches!(store.registrations(),Err(StoreError::UncertainCommit)));
        assert!(matches!(store.create_registered(spec("later"),&cap),Err(StoreError::UncertainCommit)));
    }

    #[test]
    fn c8registry_status_does_not_wait_for_fsync_or_publish_a_capability_early() {
        let scratch=Scratch::new("registry-blocked");let (entered,seen)=mpsc::channel();let (release,wait)=mpsc::channel();
        let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),
            Arc::new(BlockingDisk { entered,release:Mutex::new(wait),block_once:AtomicBool::new(true) })).unwrap());
        let (sent,received)=mpsc::channel();let writer_store=store.clone();
        let writer=std::thread::spawn(move||sent.send(writer_store.register_catalog(registry_scope(),fresh_registry_catalog("one"))).unwrap());
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let (read_sent,read_received)=mpsc::channel();let reader_store=store.clone();
        let reader=std::thread::spawn(move||read_sent.send(reader_store.registrations()).unwrap());
        let observed=read_received.recv_timeout(Duration::from_millis(300));
        let early=received.try_recv().is_ok();
        release.send(()).unwrap();writer.join().unwrap();reader.join().unwrap();
        assert!(observed.is_ok(),"registry status waited on the persistence mutex");
        assert!(observed.unwrap().unwrap().is_empty());
        assert!(!early,"registration capability escaped before persistence completed");
        let cap=received.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert!(cap.check_live().is_ok());assert!(store.registrations().unwrap()[0].usable_in_this_run);
    }

    #[test]
    fn c8registry_corrupt_catalog_reference_refuses_repeatedly_without_overwrite() {
        let scratch=Scratch::new("registry-corrupt");
        { let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
          let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();registered_id(&store,&cap,"one"); }
        let path=scratch.0.join(DATA);let mut image:serde_json::Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        image["registry"]["catalogs"]=serde_json::json!({});let corrupt=serde_json::to_vec(&image).unwrap();fs::write(&path,&corrupt).unwrap();
        for _ in 0..2 {
            assert!(matches!(PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()),Err(StoreError::Invalid(_))));
            assert_eq!(fs::read(&path).unwrap(),corrupt,"a rejected registry input was overwritten");
        }
    }

    #[test]
    fn c8registry_catalog_limits_refuse_before_any_persistence() {
        let scratch=Scratch::new("registry-bounds");let disk=Arc::new(CountingRegistryDisk(AtomicU64::new(0)));
        let mut r=registry_limits();r.max_declarations=1;r.max_catalog_bytes=512;
        let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(r),disk.clone()).unwrap();
        assert!(store.register_catalog(registry_scope(),fresh_registry_catalog(&"x".repeat(5000))).is_err());
        assert_eq!(disk.0.load(Ordering::SeqCst),0,"oversized catalog reached the persistence boundary");
        assert!(store.registrations().unwrap().is_empty());
        let other=Scratch::new("registry-declaration-bound");let mut r=registry_limits();r.max_declarations=1;
        let declaration_disk=Arc::new(CountingRegistryDisk(AtomicU64::new(0)));
        let bounded=PendingStore::open_registry_with(&other.0,11,limits(),Some(r),declaration_disk.clone()).unwrap();
        let catalog=Arc::new(crate::policy_loader::load_catalog(Some(&serde_json::json!({"a":{"cmd":"a"},"b":{"cmd":"b"}})),
            &serde_json::json!({}),15,crate::receiver::RegistrationOrigin::ClaimRole,true).unwrap());
        assert!(matches!(bounded.register_catalog(registry_scope(),catalog),Err(StoreError::Capacity("catalog declarations"))),
            "too many declarations escaped the catalog count limit");
        assert_eq!(declaration_disk.0.load(Ordering::SeqCst),0);
    }

    #[test]
    fn c8registry_registration_counter_exhaustion_never_wraps_or_overwrites() {
        let scratch=Scratch::new("registry-counter");
        { let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
          store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap(); }
        let path=scratch.0.join(DATA);let mut image:serde_json::Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        image["registry"]["next_registration_seq"]=serde_json::json!(u64::MAX);
        let bytes=serde_json::to_vec(&image).unwrap();fs::write(&path,&bytes).unwrap();
        let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
        assert!(matches!(store.register_catalog(registry_scope(),fresh_registry_catalog("one")),Err(StoreError::Capacity("registration sequence exhausted"))));
        assert_eq!(fs::read(&path).unwrap(),bytes,"exhausted registration counter overwrote the archive");
        assert!(!store.registrations().unwrap()[0].usable_in_this_run);
    }

    #[test]
    fn c8registry_explicit_recovery_retirement_prevents_restart_capacity_leak() {
        let scratch=Scratch::new("registry-restart-retire");let mut r=registry_limits();r.max_catalogs=1;r.max_registrations=1;
        let mut previous=String::new();
        for cycle in 1..=5 {
            let store=PendingStore::open_with_registry(&scratch.0,11,limits(),r).unwrap();
            if cycle>1 {
                let before=fs::read(scratch.0.join(DATA)).unwrap();
                assert!(matches!(store.register_catalog(registry_scope(),fresh_registry_catalog("one")),Err(StoreError::Capacity(_))),
                    "restart must not silently discard an open historical pin");
                assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
                assert!(matches!(store.retire_recovered_registration(&previous),Ok(PersistResult::Stored(_))));
                assert!(store.registrations().unwrap().is_empty(),"explicit recovery retirement did not reclaim an unreferenced pin");
                assert!(matches!(store.retire_recovered_registration(&previous),Ok(PersistResult::Identical)));
            }
            let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
            assert_eq!(cap.id(),format!("rb-{cycle}"),"retirement cannot reset registration high-water");
            assert!(cap.check_live().is_ok());previous=cap.id().to_owned();
        }
    }

    #[test]
    fn c8registry_recovery_retirement_preserves_pending_request_and_catalog() {
        let scratch=Scratch::new("registry-recovery-reference");let mut r=registry_limits();r.max_catalogs=1;r.max_registrations=1;
        let registration;let request;let catalog;
        { let store=PendingStore::open_with_registry(&scratch.0,11,limits(),r).unwrap();
          let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
          registration=cap.id().to_owned();catalog=cap.record.catalog_id.clone();request=registered_id(&store,&cap,"pending"); }
        let store=PendingStore::open_with_registry(&scratch.0,11,limits(),r).unwrap();
        let before=store.inspect_exact_record(&request).unwrap();
        store.retire_recovered_registration(&registration).unwrap();
        let pins=store.registrations().unwrap();assert_eq!(pins.len(),1);
        assert!(pins[0].closed&&!pins[0].usable_in_this_run,"recovery retirement is data revocation only");
        assert_eq!(store.inspect_exact_record(&request).unwrap(),before,"retirement altered a pending request");
        assert!(store.inspect_private_catalog(&catalog).is_ok(),"retirement evicted a pending request's catalog");
        assert!(matches!(store.join_existing(&before),Ok(Admission::RecoveredOnly)));
        assert!(matches!(store.register_catalog(registry_scope(),fresh_registry_catalog("new")),Err(StoreError::Capacity(_))));
    }

    #[test]
    fn c8registry_recovery_retirement_is_retriable_and_excludes_current_ids() {
        let scratch=Scratch::new("registry-recovery-failure");let old;
        { let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
          old=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap().id().to_owned(); }
        let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
        let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
        let current=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        assert!(matches!(store.retire_recovered_registration(current.id()),Err(StoreError::Conflict)),"recovery revocation accepted a current-run ID");
        assert!(current.check_live().is_ok());
        assert!(matches!(store.retire_recovered_registration("missing"),Err(StoreError::Conflict)));
        let before=fs::read(scratch.0.join(DATA)).unwrap();disk.fail.store(true,Ordering::Release);
        assert!(matches!(store.retire_recovered_registration(&old),Err(StoreError::Io { .. })));
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before,"failed retirement overwrote persisted state");
        assert!(!store.registrations().unwrap().iter().find(|r|r.id==old).unwrap().closed);
        disk.fail.store(false,Ordering::Release);store.retire_recovered_registration(&old).unwrap();
        assert!(store.registrations().unwrap().iter().all(|r|r.id!=old));
        assert!(current.check_live().is_ok(),"retirement revoked an unrelated current capability");
    }

    #[test]
    fn c8registry_busy_close_revokes_authority_without_waiting_for_fsync() {
        let scratch=Scratch::new("registry-busy-close");let (entered,seen)=mpsc::channel();let (release,wait)=mpsc::channel();
        let disk=Arc::new(BlockingDisk { entered,release:Mutex::new(wait),block_once:AtomicBool::new(false) });
        let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("one")).unwrap();
        disk.block_once.store(true,Ordering::Release);let writer_store=store.clone();let writer_cap=cap.clone();
        let writer=std::thread::spawn(move||writer_store.create_registered(spec("blocked"),&writer_cap));
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let (sent,received)=mpsc::channel();let closer_store=store.clone();let closer_cap=cap.clone();
        let closer=std::thread::spawn(move||sent.send(closer_store.close_registration(&closer_cap)).unwrap());
        let result=received.recv_timeout(Duration::from_millis(300));
        let live=cap.check_live().is_ok();let visible=store.registrations().unwrap()[0].usable_in_this_run;
        release.send(()).unwrap();let late_admission=writer.join().unwrap().unwrap();closer.join().unwrap();
        assert!(matches!(result,Ok(Err(StoreError::Busy))),"busy close waited for persistence or claimed to commit");
        assert!(!live&&!visible,"busy close retained live registration authority");
        assert!(matches!(late_admission,Admission::NewlyStored { .. }));
        assert!(cap.check_live().is_err(),"a prevalidated late storage commit revived authority");
        store.close_registration(&cap).unwrap();let pins=store.registrations().unwrap();
        assert!(pins[0].closed&&!pins[0].usable_in_this_run); // pending request still protects the catalog
    }
// Codec/shape/capacity and denial/reopen controls for the retained protocol.
// Pure codec/shape/capacity and denial/reopen specimens. NO successful native,
// ingress or edit capability is fabricated. Positive activation tests need the
// real production adapters listed in INTEGRATION-TESTS.md.

fn protocol_row_data() -> Row {
    let plan=Plan::paste("private body\n한글").with_clear(37).with_submit(421,159);
    Row { attempts:None, record:PendingRecord {
        request_id:"wb-1".into(),registration_id:Some("registration-fixture".into()),
        obligation_id:"logical-original".into(),source:SourceRef { kind:SourceKind::Direct,
            ids:vec!["source-z".into(),"source-a".into()] },
        target:Target { mode:TargetMode::Role,surface_id:7,role:Some("worker".into()) },
        lineage:RecordLineage::Protocol { data:ProtocolData {
            ingress:ImmutableIngress { intent:InputIntent::AgentText,target_kind:TargetKind::Agent,
                scope:IngressScope { socket_path:std::env::temp_dir().join("socket-fixture"),daemon_instance:11,
                    surface_instance:12,launch:13,conversation:14 },origin:"protocol-fixture".into(),
                caller:Some(CallerRecord::Surface { id:8 }),submit:SubmitContract::OwnedAtAdmission },
            initial:InitialCatalog { registration_id:"registration-fixture".into(),catalog_id:"catalog-fixture".into(),
                contract_key:"contract-fixture".into(),policy_epoch:15 },selected:None,revision:1,selected_reserve:2048 } },
        original_body_digest:"original-body-digest".into(),received_epoch_ms:1790000000123,
        plan:retained_plan(&plan),
    },terminal:None,settled:None }
}
fn protocol_data(row:&Row)->&ProtocolData {
    let RecordLineage::Protocol { data }=&row.record.lineage else { panic!("fixture branch") };data
}
fn protocol_data_mut(row:&mut Row)->&mut ProtocolData {
    let RecordLineage::Protocol { data }=&mut row.record.lineage else { panic!("fixture branch") };data
}
fn selected_data()->SelectedLineage {
    SelectedLineage { catalog_id:"catalog-fixture".into(),adapter:"claude".into(),config_digest:"config-fixture".into(),
        epochs:Epochs { daemon_instance:11,surface_instance:12,launch:13,conversation:14,policy:15,binding:16 },
        class:ProofClass::FullRequired,weak_risk:None,publication_ref:"publication-fixture".into() }
}

#[test]
fn c8protocol_exact_plan_projection_preserves_modes_and_spacing() {
    let plans=[Plan::raw(""),Plan::raw("x\n"),Plan::paste("a\u{1b}[201~b").with_clear(19).with_submit(27,31),
        Plan::control(vec![0,27,255]),Plan::submit_only(77)];
    for plan in plans {
        let exact=retained_plan(&plan);
        assert!(plan_matches(&exact,&plan));
        let roundtrip:ExactPlan=serde_json::from_slice(&serde_json::to_vec(&exact).unwrap()).unwrap();
        assert_eq!(roundtrip,exact);
        let mut wrong=exact.clone();wrong.clear_settle_ms=wrong.clear_settle_ms.wrapping_add(1);
        assert!(!plan_matches(&wrong,&plan),"timing is part of exact Plan even when output bytes match");
    }
    assert_ne!(retained_plan(&Plan::raw("")),retained_plan(&Plan::control(Vec::new())),
        "empty body and empty control are distinct executable plans");
}
#[test]
fn c8protocol_unselected_ingress_retains_catalog_without_a_fake_binding() {
    let row=protocol_row_data();validate_record(&row.record).unwrap();
    assert_eq!(row.record.visible_epochs(),None);
    assert_eq!(row.record.admission_scope(),registry_scope());
    let back:Row=serde_json::from_slice(&serde_json::to_vec(&row).unwrap()).unwrap();
    assert_eq!(back,row);
    assert_eq!(back.record.source.ids,vec!["source-z","source-a"]);
    assert_eq!(back.record.target.role.as_deref(),Some("worker"));
    assert_eq!(protocol_data(&back).ingress.scope.socket_path,std::env::temp_dir().join("socket-fixture"));
    assert!(!format!("{:?}",back.record).contains("socket-fixture"));
    assert!(!format!("{:?}",back.record).contains("private body"));
}
#[test]
fn c8protocol_production_socket_paths_satisfy_ingress_shape() {
    for socket_path in [cys::default_socket_path(),cys::dept_socket_path("dept-5")] {
        assert!(socket_path.is_absolute(),"production socket path must be absolute: {socket_path:?}");
        let mut row=protocol_row_data();
        protocol_data_mut(&mut row).ingress.scope.socket_path=socket_path;
        protocol_shape(&row.record,protocol_data(&row)).unwrap();
        validate_record(&row.record).unwrap();
    }
    let mut relative=protocol_row_data();
    protocol_data_mut(&mut relative).ingress.scope.socket_path=PathBuf::from("relative/socket-fixture");
    assert!(matches!(protocol_shape(&relative.record,protocol_data(&relative)),
        Err(StoreError::Invalid("protocol ingress identity"))));
}
#[test]
fn c8protocol_shape_refuses_other_intents_without_claiming_ingress_authority() {
    for intent in [InputIntent::HumanTerminal,InputIntent::UserPathInsertion,
        InputIntent::TerminalProtocol,InputIntent::TerminalDisplayControl] {
        let mut row=protocol_row_data();protocol_data_mut(&mut row).ingress.intent=intent;
        assert!(matches!(validate_record(&row.record),Err(StoreError::Invalid("intent outside durable automatic ingress"))));
    }
    let mut row=protocol_row_data();protocol_data_mut(&mut row).ingress.target_kind=TargetKind::Shell;
    assert!(validate_record(&row.record).is_err());
    protocol_data_mut(&mut row).ingress.intent=InputIntent::LifecycleLaunch;
    assert!(validate_record(&row.record).is_ok(),"valid retained lifecycle DATA is not an executable lifecycle grant");
}
#[test]
fn c8protocol_selection_shape_checks_all_fixed_scope_axes() {
    let row=protocol_row_data();let p=protocol_data(&row);let good=selected_data();
    selected_shape(&row.record,p,&good).unwrap();
    for axis in 0..8 {
        let mut bad=good.clone();match axis {
            0=>bad.epochs.daemon_instance+=1,1=>bad.epochs.surface_instance+=1,2=>bad.epochs.launch+=1,
            3=>bad.epochs.conversation+=1,4=>bad.epochs.policy+=1,5=>bad.epochs.binding=0,
            6=>bad.catalog_id.push('x'),7=>bad.publication_ref.clear(),_=>unreachable!() }
        assert!(selected_shape(&row.record,p,&bad).is_err(),"mismatched axis {axis}");
    }
    let mut weak=good;weak.class=ProofClass::ObservedAgentWeak;
    assert!(selected_shape(&row.record,p,&weak).is_err());
    weak.weak_risk=Some(WeakRisk::DeclaredCustomInvocation);
    assert!(selected_shape(&row.record,p,&weak).is_ok(),"weak population is retained, not globally rejected");
}
#[test]
fn c8protocol_association_window_is_retained_with_original_ingress() {
    let mut row=protocol_row_data();row.record.plan=retained_plan(&Plan::paste("await original Return"));
    protocol_data_mut(&mut row).ingress.submit=SubmitContract::AwaitingAssociation {
        window_ms:crate::submit_assoc::LEGACY_ASSOCIATION_WINDOW_MS };
    validate_record(&row.record).unwrap();
    let back:Row=serde_json::from_slice(&serde_json::to_vec(&row).unwrap()).unwrap();
    assert_eq!(back,row);
    protocol_data_mut(&mut row).ingress.caller=None;
    assert!(matches!(validate_record(&row.record),Err(StoreError::Invalid("submit contract does not match exact plan"))));
}
#[test]
fn c8protocol_selection_and_revision_growth_use_admission_headroom() {
    let row=protocol_row_data();let charged=checked_row_charge(&row,limits()).unwrap();
    let mut selected=row.clone();let p=protocol_data_mut(&mut selected);
    p.selected=Some(selected_data());p.revision=u64::MAX;
    assert_eq!(checked_row_charge(&selected,limits()).unwrap(),charged,
        "selected data + widest revision spend reserved bytes, not new capacity");
    let mut too_small=row.clone();protocol_data_mut(&mut too_small).selected_reserve=limits().max_record_bytes;
    assert!(matches!(checked_row_charge(&too_small,limits()),Err(StoreError::Capacity(_))),
        "unfundable future selected lineage is refused before admission");
    let mut oversized=row;protocol_data_mut(&mut oversized).selected=Some(selected_data());
    protocol_data_mut(&mut oversized).selected.as_mut().unwrap().publication_ref="x".repeat(4096);
    assert!(matches!(checked_row_charge(&oversized,limits()),Err(StoreError::Capacity("encoded bytes"))));
}
#[test]
fn c8protocol_schema3_rejects_without_overwrite_or_migration() {
    let scratch=Scratch::new("protocol-schema3");
    let bytes=serde_json::to_vec(&Snapshot { schema:3,next_request_seq:1,next_settle_seq:1,
        rows:vec![],registry:RegistryImage::default() }).unwrap();
    write_private(&scratch.0.join(DATA),&bytes);
    assert!(matches!(PendingStore::open(&scratch.0,11,limits()),Err(StoreError::Invalid("schema"))));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),bytes);
}

// Stored JSON fixture only. It is deliberately created while the store is CLOSED
// and then recovered. This must never produce CurrentRequest/native authority.
#[test]
fn c8protocol_reopen_preserves_exact_data_but_mints_no_current_request() {
    let scratch=Scratch::new("protocol-recover");let mut row=protocol_row_data();let registry;
    {
        let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();
        row.record.registration_id=Some(cap.id().into());
        let p=protocol_data_mut(&mut row);p.initial.registration_id=cap.id().into();
        p.initial.catalog_id=cap.record.catalog_id.clone();p.initial.contract_key=cap.record.contract_key.clone();
        registry=store.try_mutation().unwrap().registry.as_ref().clone();
    }
    let bytes=serde_json::to_vec(&Snapshot { schema:SCHEMA,next_request_seq:2,next_settle_seq:1,
        rows:vec![row.clone()],registry }).unwrap();
    fs::write(scratch.0.join(DATA),&bytes).unwrap();
    let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
    assert_eq!(store.inspect_exact_record(&row.record.request_id).unwrap(),row.record);
    assert!(matches!(store.join_existing(&row.record),Ok(Admission::RecoveredOnly)));
    assert!(store.try_mutation().unwrap().current_requests.is_empty());
    let status=store.status(&row.record.request_id).unwrap().unwrap();
    assert!(status.recovered);assert_eq!(status.epochs,None);
    assert!(matches!(store.record_terminal(&row.record.request_id,terminal(&row.record.request_id,RetiredReason::WrittenSubmitted)),
        Err(StoreError::Invalid("typed protocol writer receipt required"))));
    assert!(matches!(store.mark_settled(&row.record.request_id,settlement(SettlementKind::ProducerSettled,1)),
        Err(StoreError::Invalid("typed protocol consumer settlement required"))));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),bytes);
}

struct RefuseProtocolIngress;
impl IngressAuthority for RefuseProtocolIngress {
    fn validate_and_consume(self,_:&NewProtocolSpec,_:&RegisteredCatalog)->Result<(),ProtocolError> {
        Err(ProtocolError::Authority("intentional denial fixture".into()))
    }
}
fn denied_protocol_spec() -> NewProtocolSpec {
    let row=protocol_row_data();let ingress=protocol_data(&row).ingress.clone();
    NewProtocolSpec { obligation_id:row.record.obligation_id,source:row.record.source,target:row.record.target,
        ingress,original_body_digest:row.record.original_body_digest,received_epoch_ms:row.record.received_epoch_ms,
        received_at:Instant::now(),plan:Plan::paste("private body\n한글").with_clear(37).with_submit(421,159),
        origin:"protocol-fixture" }
}
#[test]
fn c8protocol_ingress_denial_does_not_advance_ids_or_publish_runtime_owner() {
    let scratch=Scratch::new("protocol-denial");
    let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();
    let before=fs::read(scratch.0.join(DATA)).unwrap();
    let seq=store.try_mutation().unwrap().next_request_seq;
    assert!(matches!(store.accept_protocol(denied_protocol_spec(),&cap,RefuseProtocolIngress,
        ProtocolLimits { max_selected_lineage_bytes:2048 }),Err(ProtocolError::Authority(_))));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
    let state=store.try_mutation().unwrap();assert_eq!(state.next_request_seq,seq);
    assert!(state.current_requests.is_empty());assert!(state.rows.is_empty());
}
#[test]
fn c8protocol_submit_is_existing_operation_only_even_with_caller_data() {
    let scratch=Scratch::new("protocol-submit");
    let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();
    let before=fs::read(scratch.0.join(DATA)).unwrap();let mut spec=denied_protocol_spec();
    spec.ingress.intent=InputIntent::AgentSubmit;spec.plan=Plan::submit_only(0);
    spec.ingress.submit=SubmitContract::BoundToCompletedBody { body_request_id:"previous-body".into(),body_lineage:selected_data() };
    assert!(matches!(store.accept_protocol(spec,&cap,RefuseProtocolIngress,
        ProtocolLimits { max_selected_lineage_bytes:2048 }),Err(ProtocolError::NeedsExistingRequest)));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
}

// Synthetic upstream authorization ASSUMPTIONS for storage/lease tests only. The grants
// compare exact captured inputs; they are not real caller/native authority or production factories.
#[derive(PartialEq,Eq)]
struct FixtureIngressFacts {
    obligation:String,source:SourceRef,target:Target,ingress:ImmutableIngress,digest:String,
    at_ms:u64,at:Instant,plan:Plan,origin:&'static str,
}
impl FixtureIngressFacts {
    fn of(s:&NewProtocolSpec)->Self { Self { obligation:s.obligation_id.clone(),source:s.source.clone(),target:s.target.clone(),
        ingress:s.ingress.clone(),digest:s.original_body_digest.clone(),at_ms:s.received_epoch_ms,at:s.received_at,
        plan:s.plan.clone(),origin:s.origin } }
}
struct FixtureIngressGrant { facts:FixtureIngressFacts,registration:Arc<RegisteredCatalog> }
impl IngressAuthority for FixtureIngressGrant {
    fn validate_and_consume(self,spec:&NewProtocolSpec,registration:&RegisteredCatalog)->Result<(),ProtocolError> {
        if self.facts!=FixtureIngressFacts::of(spec) || !std::ptr::eq(self.registration.as_ref(),registration) {
            return Err(ProtocolError::Authority("fixture ingress inputs changed".into()));
        }
        registration.check_live()?;Ok(())
    }
}
struct FixtureSelectionGrant {
    current:Arc<CurrentRequest>,data:ProtocolData,selected:SelectedLineage,calls:Arc<AtomicU64>,
}
impl InitialSelectionAuthority for FixtureSelectionGrant {
    fn validate_and_consume(self,request:&Arc<Request>,plan:&Plan,ingress:&ImmutableIngress,
        initial:&InitialCatalog,expected:u64)->Result<SelectedLineage,ProtocolError> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        if !Arc::ptr_eq(request,self.current.request()) || plan!=self.current.plan() || ingress!=&self.data.ingress
            || initial!=&self.data.initial || expected!=self.data.revision {
            return Err(ProtocolError::Authority("fixture selection inputs changed".into()));
        }
        Ok(self.selected)
    }
}
fn fixture_protocol_admit(store:&PendingStore,cap:&Arc<RegisteredCatalog>,at:Instant)->Arc<CurrentRequest> {
    let mut spec=denied_protocol_spec();spec.received_at=at;
    let grant=FixtureIngressGrant { facts:FixtureIngressFacts::of(&spec),registration:cap.clone() };
    store.accept_protocol(spec,cap,grant,ProtocolLimits { max_selected_lineage_bytes:2048 }).unwrap()
}
fn fixture_selection(store:&PendingStore,current:&Arc<CurrentRequest>,revision:u64)->FixtureSelectionGrant {
    let data=store.current_revision(current,revision).unwrap();
    let contract=current.initial_registration.catalog().unwrap().get(Some("custom")).unwrap();let p=&contract.policy;
    let selected=SelectedLineage { catalog_id:data.initial.catalog_id.clone(),adapter:p.adapter.clone(),config_digest:p.config_digest.clone(),
        epochs:Epochs { daemon_instance:data.ingress.scope.daemon_instance,surface_instance:data.ingress.scope.surface_instance,
            launch:data.ingress.scope.launch,conversation:data.ingress.scope.conversation,policy:data.initial.policy_epoch,binding:16 },
        class:p.class,weak_risk:p.weak_risk,publication_ref:"SYNTHETIC_UPSTREAM_SELECTION".into() };
    FixtureSelectionGrant { current:current.clone(),data,selected,calls:Arc::new(AtomicU64::new(0)) }
}
#[test]
fn c8protocol_live_owner_selection_commits_same_request_plan_and_age() {
    let scratch=Scratch::new("protocol-current");let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();
    let at=Instant::now()-Duration::from_secs(7);let current=fixture_protocol_admit(&store,&cap,at);let original=current.request().clone();let plan=current.plan().clone();
    assert_eq!(original.created,at);assert!(store.current_revision(&current,1).unwrap().selected.is_none());
    let before=store.inspect_exact_record(current.id()).unwrap();
    let grant=fixture_selection(&store,&current,1);let selected=grant.selected.clone();let edit=original.reserve_zero_write_edit().unwrap();
    assert_eq!(store.select_initial_lineage(&current,1,edit,grant).unwrap(),2);
    assert!(Arc::ptr_eq(&original,current.request()));assert_eq!(current.plan(),&plan);assert_eq!(current.received_at(),at);
    let after=store.inspect_exact_record(current.id()).unwrap();assert_eq!(after.plan,before.plan);assert_eq!(after.source,before.source);
    assert_eq!(after.original_body_digest,before.original_body_digest);assert_eq!(after.received_epoch_ms,before.received_epoch_ms);
    let image:Snapshot=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
    let stored=image.rows.iter().find(|r|r.record.request_id==current.id()).unwrap();
    assert_eq!(protocol_data(stored).selected,Some(selected));assert_eq!(protocol_data(stored).revision,2);
    assert!(matches!(store.current_revision(&current,1),Err(ProtocolError::RevisionChanged)));
    assert!(original.reserve_attempt(&plan,None).is_ok(),"committed same-plan edit did not release its reservation");
}
#[test]
fn c8protocol_idempotent_selection_requires_a_reserved_edit() {
    let scratch=Scratch::new("protocol-idempotent");let store=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();let current=fixture_protocol_admit(&store,&cap,Instant::now());
    store.select_initial_lineage(&current,1,current.request().reserve_zero_write_edit().unwrap(),fixture_selection(&store,&current,1)).unwrap();
    let before=fs::read(scratch.0.join(DATA)).unwrap();
    assert_eq!(store.select_initial_lineage(&current,2,current.request().reserve_zero_write_edit().unwrap(),fixture_selection(&store,&current,2)).unwrap(),2);
    let mut edit=current.request().reserve_zero_write_edit().unwrap();let _stamp=edit.begin_persisting(current.plan()).unwrap();
    let grant=fixture_selection(&store,&current,2);let calls=grant.calls.clone();let result=store.select_initial_lineage(&current,2,edit,grant);
    assert!(result.is_err()&&calls.load(Ordering::SeqCst)==0,"an unresolved Persisting edit reached authority or reported idempotent success");
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);assert_eq!(store.current_revision(&current,2).unwrap().revision,2);
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(crate::agent_write::LeaseError::Busy)),
        "refusing a pre-Persisting edit released its unresolved exclusion");
}
#[test]
fn c8protocol_proved_preinstall_failure_resolves_edit_without_changing_data() {
    let scratch=Scratch::new("protocol-nochange");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();let current=fixture_protocol_admit(&store,&cap,Instant::now());
    let before=fs::read(scratch.0.join(DATA)).unwrap();disk.fail.store(true,Ordering::Release);
    let result=store.select_initial_lineage(&current,1,current.request().reserve_zero_write_edit().unwrap(),fixture_selection(&store,&current,1));
    assert!(matches!(result,Err(ProtocolError::Store(StoreError::Io { installed:false,.. }))));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);assert!(store.current_revision(&current,1).unwrap().selected.is_none());
    let lease=current.request().reserve_attempt(current.plan(),None).expect("proved pre-install no-change must resolve its own edit");drop(lease);
    disk.fail.store(false,Ordering::Release);
    assert_eq!(store.select_initial_lineage(&current,1,current.request().reserve_zero_write_edit().unwrap(),fixture_selection(&store,&current,1)).unwrap(),2);
}
#[test]
fn c8protocol_uncertain_selection_keeps_edit_and_recovers_only_data() {
    let scratch=Scratch::new("protocol-uncertain");let disk=Arc::new(PanicOnceAfterReplace(AtomicBool::new(false)));
    let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();let current=fixture_protocol_admit(&store,&cap,Instant::now());
    let grant=fixture_selection(&store,&current,1);let edit=current.request().reserve_zero_write_edit().unwrap();disk.0.store(true,Ordering::Release);
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||store.select_initial_lineage(&current,1,edit,grant)));
    assert!(result.is_err());assert!(matches!(store.current_revision(&current,1),Err(ProtocolError::Store(StoreError::UncertainCommit))));
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(crate::agent_write::LeaseError::Busy)),
        "uncertain selected replacement released its native edit reservation");
    let image:Snapshot=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
    assert_eq!(protocol_data(&image.rows[0]).revision,2,"premise: candidate really reached disk before unwind");
    drop(store);let reopened=PendingStore::open_with_registry(&scratch.0,11,limits(),registry_limits()).unwrap();
    assert!(reopened.try_mutation().unwrap().current_requests.is_empty());
    assert!(matches!(reopened.current_revision(&current,2),Err(ProtocolError::Inactive)));
    assert!(reopened.status(current.id()).unwrap().unwrap().recovered);
}
#[test]
fn c8protocol_failed_admission_grants_no_current_request_or_new_highwater() {
    let scratch=Scratch::new("protocol-admission-failure");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();let before=fs::read(scratch.0.join(DATA)).unwrap();
    let at=Instant::now()-Duration::from_secs(9);let mut spec=denied_protocol_spec();spec.received_at=at;
    let grant=FixtureIngressGrant { facts:FixtureIngressFacts::of(&spec),registration:cap.clone() };disk.fail.store(true,Ordering::Release);
    assert!(matches!(store.accept_protocol(spec,&cap,grant,ProtocolLimits { max_selected_lineage_bytes:2048 }),Err(ProtocolError::Store(StoreError::Io { installed:false,.. }))));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);
    { let state=store.try_mutation().unwrap();assert!(state.current_requests.is_empty());assert_eq!(state.next_request_seq,1); }
    disk.fail.store(false,Ordering::Release);let current=fixture_protocol_admit(&store,&cap,at);
    assert_eq!(current.id(),"wb-1");assert_eq!(current.request().created,at);
}
#[test]
fn c8protocol_selection_fsync_holds_exclusion_without_native_mutex_or_early_data() {
    let scratch=Scratch::new("protocol-select-fsync");let (entered,seen)=mpsc::channel();let (release,wait)=mpsc::channel();
    let disk=Arc::new(BlockingDisk { entered,release:Mutex::new(wait),block_once:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();let current=fixture_protocol_admit(&store,&cap,Instant::now());
    let grant=fixture_selection(&store,&current,1);let edit=current.request().reserve_zero_write_edit().unwrap();
    disk.block_once.store(true,Ordering::Release);let writer_store=store.clone();let writer_current=current.clone();
    let writer=std::thread::spawn(move||writer_store.select_initial_lineage(&writer_current,1,edit,grant));seen.recv_timeout(Duration::from_secs(5)).unwrap();
    let reader_store=store.clone();let reader_current=current.clone();let (sent,received)=mpsc::channel();
    let reader=std::thread::spawn(move||sent.send((reader_store.status(reader_current.id()),reader_current.request().native_progress(),
        matches!(reader_current.request().reserve_attempt(reader_current.plan(),None),Err(crate::agent_write::LeaseError::Busy)))).unwrap());
    let observed=received.recv_timeout(Duration::from_millis(300));release.send(()).unwrap();let result=writer.join().unwrap();reader.join().unwrap();
    assert!(observed.is_ok(),"selection fsync blocked status/native progress on a held native mutex");
    let (status,progress,busy)=observed.unwrap();assert!(status.unwrap().unwrap().epochs.is_none()&&progress.proven_zero()&&busy,
        "selection published early or released the actual request exclusion before fsync");
    assert_eq!(result.unwrap(),2);assert!(current.request().reserve_attempt(current.plan(),None).is_ok());
}

#[test]
fn c8protocol_postinstall_error_cannot_mint_not_written_resolution() {
    struct InstalledError(AtomicBool);
    impl Disk for InstalledError {
        fn replace(&self,root:&Path,bytes:&[u8])->Result<Durability,CommitFailure> {
            let level=RealDisk.replace(root,bytes)?;
            if self.0.load(Ordering::Acquire) { Err(CommitFailure { stage:"fixture_after_install",installed:true,uncertain:true,
                message:"controlled post-install error".into() }) } else { Ok(level) }
        }
    }
    let scratch=Scratch::new("protocol-installed-error");let disk=Arc::new(InstalledError(AtomicBool::new(false)));
    let store=PendingStore::open_registry_with(&scratch.0,11,limits(),Some(registry_limits()),disk.clone()).unwrap();
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("protocol")).unwrap();let current=fixture_protocol_admit(&store,&cap,Instant::now());
    let grant=fixture_selection(&store,&current,1);let edit=current.request().reserve_zero_write_edit().unwrap();disk.0.store(true,Ordering::Release);
    let result=store.select_initial_lineage(&current,1,edit,grant);
    assert!(matches!(result,Err(ProtocolError::Store(StoreError::Io { installed:true,.. }))));
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(crate::agent_write::LeaseError::Busy)),
        "post-install error was treated as proved not-written and released its edit");
    assert!(matches!(store.current_revision(&current,1),Err(ProtocolError::Store(StoreError::UncertainCommit))));
    let disk:Snapshot=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();assert_eq!(protocol_data(&disk.rows[0]).revision,2);
}

// Real store + writer IO/receipt tests; caller/native authority is the explicitly
// synthetic upstream fixture used by the preceding protocol controls.
fn attempt_limits()->attempts::AttemptLimits { attempts::AttemptLimits { max_records:3,max_record_bytes:8192,diagnostic_preview_bytes:64 } }
fn attempt_store_limits()->Limits { Limits { max_record_bytes:65536,max_file_bytes:1048576,max_recent_bytes:196608,..limits() } }
fn attempt_current(store:&PendingStore,cap:&Arc<RegisteredCatalog>,plan:Plan)->Arc<CurrentRequest> {
    let mut spec=denied_protocol_spec();spec.plan=plan;
    spec.ingress.intent=if spec.plan.exact_view().control.is_some() {InputIntent::AgentControl}else{InputIntent::AgentText};
    spec.ingress.submit=if spec.plan.owns_submit() { SubmitContract::OwnedAtAdmission }
        else if spec.ingress.intent==InputIntent::AgentControl { SubmitContract::NotApplicable }
        else { SubmitContract::SelfContainedInBody };
    let grant=FixtureIngressGrant { facts:FixtureIngressFacts::of(&spec),registration:cap.clone() };
    let current=store.accept_protocol(spec,cap,grant,ProtocolLimits { max_selected_lineage_bytes:2048 }).unwrap();
    store.select_initial_lineage(&current,1,current.request().reserve_zero_write_edit().unwrap(),fixture_selection(store,&current,1)).unwrap();current
}
fn attempt_revision(store:&PendingStore,current:&Arc<CurrentRequest>)->u64 {
    match store.inspect_exact_record(current.id()).unwrap().lineage { RecordLineage::Protocol { data }=>data.revision,_=>panic!("protocol") }
}
fn attempt_prepare(store:&Arc<PendingStore>,current:&Arc<CurrentRequest>,resume:Option<crate::agent_write::Envelope>)->attempts::PreparedAttempt {
    let lease=current.request().reserve_attempt(current.plan(),resume).unwrap();
    store.prepare_attempt(current,attempt_revision(store,current),lease,resume,attempt_limits()).unwrap()
}
fn attempt_execute<W:std::io::Write>(writer:&mut W,job:&crate::agent_write::AgentJob)->crate::agent_write::Executed {
    let mut last=None;let mut first=||{};let mut body=|_:&str|{};
    let mut io=crate::agent_write::WriterIo { sleep:&|_|{},last_program_write:&mut last,on_first_byte:&mut first,on_body_written:&mut body };
    crate::agent_write::execute(writer,job,&mut io)
}
struct AttemptRefusal;
impl crate::agent_write::Proof for AttemptRefusal {
    fn admit(&self)->Result<(),crate::receiver::ProofLoss>{Err(crate::receiver::ProofLoss::Unknown("fixture retry".into()))}
    fn recheck(&self,_:crate::agent_write::Phase)->Result<(),crate::receiver::ProofLoss>{self.admit()}
}
struct AttemptCut(std::sync::atomic::AtomicUsize);
impl crate::agent_write::Proof for AttemptCut {
    fn admit(&self)->Result<(),crate::receiver::ProofLoss>{self.0.store(0,Ordering::SeqCst);Ok(())}
    fn recheck(&self,_:crate::agent_write::Phase)->Result<(),crate::receiver::ProofLoss>{
        if self.0.fetch_add(1,Ordering::SeqCst)==0 { Err(crate::receiver::ProofLoss::Unknown("fixture exact cut".into())) } else { Ok(()) }
    }
}
#[test]
fn c8attempt_real_writer_commits_exact_start_and_outcome_before_source_closure() {
    use crate::agent_write::{self,PassThrough,Terminal};
    let scratch=Scratch::new("attempt-complete");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();
    let current=attempt_current(&store,&cap,Plan::paste("body 한글").with_submit(0,0));let p=attempt_prepare(&store,&current,None);let journal=p.journal();
    let rows=store.attempt_summaries(current.id()).unwrap();assert_eq!(rows.len(),1);assert_eq!(rows[0].phase,attempts::AttemptPhase::Prepared);
    let job=p.into_job(Arc::new(PassThrough)).unwrap();let expected=job.plan.planned_bytes();let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);
    assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].phase,attempts::AttemptPhase::Writing);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(written,expected);
    let rows=store.attempt_summaries(current.id()).unwrap();assert_eq!(rows[0].outcome,Some(attempts::OutcomeKind::WrittenSubmitted));
    assert!(rows[0].native.unwrap().envelope.submitted&&journal.has_stored_outcome());
    assert!(store.inspect_settlement(current.id()).unwrap().is_none(),"writer completion is not durable source settlement");
}
#[test]
fn c8attempt_wrong_plan_and_legacy_journal_entry_cannot_create_writing() {
    use crate::agent_write::{self,PassThrough,Journal,Terminal,Envelope};
    let scratch=Scratch::new("attempt-context");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();assert!(journal.writing(current.request()).is_err());
    assert!(journal.terminal(current.request(),&Terminal::WrittenSubmitted { accepted:Default::default() },&Envelope::default()).is_err());
    let mut job=p.into_job(Arc::new(PassThrough)).unwrap();let original=job.plan.clone();job.plan=Plan::raw("different").with_submit(0,0);
    let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);assert!(matches!(ex.terminal,Terminal::PartialUnknown{..})&&written.is_empty());
    assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].phase,attempts::AttemptPhase::Prepared);
    job.plan=original;let ex=attempt_execute(&mut written,&job);assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(written,b"owned\r");
}
#[test]
fn c8attempt_partial_chain_preserves_first_anchor_and_reclaims_only_consumed_history() {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new("attempt-resume");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let body="x".repeat(10000);
    let current=attempt_current(&store,&cap,Plan::raw(&body).with_submit(0,0));let proof=Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)));
    let mut written=Vec::new();let mut resume=None;let mut numbers=Vec::new();
    for _ in 0..4 {
        let p=attempt_prepare(&store,&current,resume);numbers.push(p.number());let job=p.into_job(proof.clone()).unwrap();
        let ex=attempt_execute(&mut written,&job);let envelope=ex.envelope;let terminal=agent_write::finish(job,&ex);
        match terminal { Terminal::PartialExact { .. }=>resume=Some(envelope),Terminal::WrittenSubmitted { .. }=>{resume=None;break},other=>panic!("unexpected {other:?}") }
    }
    assert!(resume.is_none());assert_eq!(numbers,vec![1,2,3,4]);assert_eq!(written,[body.as_bytes(),b"\r"].concat());
    let summaries=store.attempt_summaries(current.id()).unwrap();assert_eq!(summaries.iter().map(|r|r.number).collect::<Vec<_>>(),vec![1,3,4]);
    let disk:serde_json::Value=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
    assert_eq!(disk["rows"][0]["attempts"]["first_partial"],1);assert_eq!(disk["rows"][0]["attempts"]["next_number"],5);
    assert_eq!(summaries.last().unwrap().native.unwrap().accepted_total,10001);
}
#[test]
fn c8attempt_many_no_write_retries_keep_one_request_and_monotonic_attempt_ids() {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new("attempt-retries");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));let id=current.id().to_owned();
    let mut written=Vec::new();for number in 1..=10 {
        let p=attempt_prepare(&store,&current,None);assert_eq!(p.number(),number);let job=p.into_job(Arc::new(AttemptRefusal)).unwrap();
        let ex=attempt_execute(&mut written,&job);assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    }
    assert!(written.is_empty());assert_eq!(current.id(),id);assert_eq!(store.inventory().unwrap().len(),1);
    assert_eq!(store.attempt_summaries(current.id()).unwrap().iter().map(|r|r.number).collect::<Vec<_>>(),vec![8,9,10]);
}
#[test]
fn c8attempt_failed_writing_and_outcome_retry_never_reexecute_or_release_implicitly() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("attempt-io-refusal");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();disk.fail.store(true,Ordering::Release);
    let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    assert!(written.is_empty()&&!journal.has_stored_outcome());assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].phase,attempts::AttemptPhase::Prepared);
    disk.fail.store(false,Ordering::Release);journal.retry_outcome().unwrap();assert!(journal.has_stored_outcome());assert!(written.is_empty());
    assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].outcome,Some(attempts::OutcomeKind::RefusedNoWrite));
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),"receipt retry must not silently grant another execution");
}
#[test]
fn c8attempt_completed_bytes_with_failed_receipt_are_retained_without_duplicate_body() {
    use crate::agent_write::{self,PassThrough,Terminal};
    struct FailAfterWrite { bytes:Vec<u8>,disk:Arc<SwitchDisk> }
    impl std::io::Write for FailAfterWrite {
        fn write(&mut self,b:&[u8])->io::Result<usize>{self.bytes.extend_from_slice(b);self.disk.fail.store(true,Ordering::Release);Ok(b.len())}
        fn flush(&mut self)->io::Result<()>{Ok(())}
    }
    let scratch=Scratch::new("attempt-late-receipt");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();let mut writer=FailAfterWrite { bytes:Vec::new(),disk:disk.clone() };
    let ex=attempt_execute(&mut writer,&job);assert!(matches!(ex.terminal,Terminal::WrittenSubmitted{..}));
    assert!(matches!(agent_write::finish(job,&ex),Terminal::PartialUnknown{..}));assert_eq!(writer.bytes,b"owned\r");
    assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].phase,attempts::AttemptPhase::Writing);
    disk.fail.store(false,Ordering::Release);journal.retry_outcome().unwrap();journal.retry_outcome().unwrap();
    assert_eq!(writer.bytes,b"owned\r");assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].outcome,Some(attempts::OutcomeKind::WrittenSubmitted));
    assert!(matches!(current.request().status(),agent_write::Status::Terminal(Terminal::PartialUnknown{..})),"storage retry cannot silently change the logical reconciliation disposition");
}
#[test]
fn c8attempt_restart_restores_records_as_data_and_cannot_restore_the_old_journal() {
    use crate::agent_write::{self,PassThrough};
    let scratch=Scratch::new("attempt-reopen");let current;let journal;let bytes;
    {
        let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
        let p=attempt_prepare(&store,&current,None);journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();let mut writer=Vec::new();let ex=attempt_execute(&mut writer,&job);agent_write::finish(job,&ex);
        bytes=fs::read(scratch.0.join(DATA)).unwrap();
    }
    assert!(journal.retry_outcome().is_err());
    let reopened=PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap();
    assert!(reopened.attempt_summaries(current.id()).unwrap()[0].recovered);assert!(reopened.try_mutation().unwrap().current_requests.is_empty());
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),bytes);
}
#[test]
fn c8attempt_completion_size_is_funded_before_prepare() {
    let scratch=Scratch::new("attempt-funding");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let before=fs::read(scratch.0.join(DATA)).unwrap();let lease=current.request().reserve_attempt(current.plan(),None).unwrap();
    let small=attempts::AttemptLimits { max_records:3,max_record_bytes:2048,diagnostic_preview_bytes:512 };
    assert!(matches!(store.prepare_attempt(&current,2,lease,None,small),Err(ProtocolError::Store(StoreError::Capacity("unfunded attempt completion")))));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);assert!(store.attempt_summaries(current.id()).unwrap().is_empty());
    assert!(current.request().native_progress().proven_zero());let valid=attempt_prepare(&store,&current,None);assert_eq!(valid.number(),1);
}
#[test]
fn c8attempt_zero_byte_transport_error_retains_the_actual_call_count() {
    use crate::agent_write::{self,PassThrough,Terminal};
    struct Broken;
    impl std::io::Write for Broken {fn write(&mut self,_:&[u8])->io::Result<usize>{Err(io::ErrorKind::BrokenPipe.into())}fn flush(&mut self)->io::Result<()>{Ok(())}}
    let scratch=Scratch::new("attempt-zero-transport");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();let ex=attempt_execute(&mut Broken,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));let rows=store.attempt_summaries(current.id()).unwrap();
    assert!(journal.has_stored_outcome());assert_eq!(rows[0].accepted.unwrap().calls,1);assert_eq!(rows[0].native.unwrap().accepted_total,0);
}

#[test]
fn c8attempt_previous_prototype_schema_is_not_silently_migrated() {
    let scratch=Scratch::new("attempt-schema4");
    let old=serde_json::to_vec(&Snapshot { schema:4,next_request_seq:1,next_settle_seq:1,rows:vec![],registry:RegistryImage::default() }).unwrap();
    write_private(&scratch.0.join(DATA),&old);
    for _ in 0..2 {
        assert!(matches!(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()),Err(StoreError::Invalid("schema"))));
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),old);
    }
}
fn attempt_byte_pressure_retries(label:&str,store_limits:Limits) {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new(label);let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,store_limits,registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let limits=attempts::AttemptLimits { max_records:64,..attempt_limits() };let mut written=Vec::new();
    for number in 1..=64 {
        let lease=current.request().reserve_attempt(current.plan(),None).unwrap();
        let result=store.prepare_attempt(&current,attempt_revision(&store,&current),lease,None,limits);
        let p=result.unwrap_or_else(|e|panic!("reclaimable consumed attempt blocked retry {number}: {e:?}"));
        assert_eq!(p.number(),number);let job=p.into_job(Arc::new(AttemptRefusal)).unwrap();
        let ex=attempt_execute(&mut written,&job);assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    }
    assert!(written.is_empty());let rows=store.attempt_summaries(current.id()).unwrap();
    assert!(rows.len()<64,"fixture must reach byte pressure before count pressure");assert_eq!(rows.last().unwrap().number,64);
    assert_eq!(store.inventory().unwrap().len(),1);
}
#[test]
fn c8attempt_row_bytes_reclaim_consumed_history_before_count_limit() {
    attempt_byte_pressure_retries("attempt-row-pressure",Limits { max_record_bytes:20480,..attempt_store_limits() });
}
#[test]
fn c8attempt_file_bytes_reclaim_consumed_history_before_count_limit() {
    attempt_byte_pressure_retries("attempt-file-pressure",Limits { max_file_bytes:65536,..attempt_store_limits() });
}
#[test]
fn c8attempt_revision_headroom_is_reserved_before_prepare_or_native_bytes() {
    use crate::agent_write::{self,PassThrough,Terminal};
    for (revision,allowed) in [(u64::MAX-2,false),(u64::MAX-3,true)] {
        let scratch=Scratch::new("attempt-revision");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
        { let mut state=store.try_mutation().unwrap();let mut rows=(*state.rows).clone();
          let RecordLineage::Protocol { data }=&mut rows.get_mut(current.id()).unwrap().record.lineage else {unreachable!()};data.revision=revision;
          let mut registry=state.registry.clone();let nr=state.next_request_seq;let ns=state.next_settle_seq;
          let bytes=prune_and_encode(&mut rows,nr,ns,store.limits,None,&mut registry,store.registry_limits).unwrap();
          store.commit(&mut state,rows,nr,ns,registry,&bytes).unwrap(); }
        let before=fs::read(scratch.0.join(DATA)).unwrap();let lease=current.request().reserve_attempt(current.plan(),None).unwrap();
        let result=store.prepare_attempt(&current,revision,lease,None,attempt_limits());
        if allowed {
            let job=result.unwrap().into_job(Arc::new(PassThrough)).unwrap();let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);
            assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(written,b"owned\r");
            assert_eq!(attempt_revision(&store,&current),u64::MAX);assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].phase,attempts::AttemptPhase::Outcome);
        } else {
            assert!(matches!(result,Err(ProtocolError::Store(StoreError::Capacity("request revision exhausted")))),"Prepared admitted without Writing+Outcome revision headroom");
            assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);assert!(current.request().native_progress().proven_zero());
            assert!(current.request().reserve_attempt(current.plan(),None).is_ok());
        }
    }
}

#[test]
fn c8attempt_byte_pressure_cannot_erase_latest_conclusive_result() {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new("attempt-protected-capacity");let limits=Limits { max_record_bytes:16384,..attempt_store_limits() };
    let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,limits,registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(AttemptRefusal)).unwrap();let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));let before=fs::read(scratch.0.join(DATA)).unwrap();
    let lease=current.request().reserve_attempt(current.plan(),None).unwrap();let result=store.prepare_attempt(&current,attempt_revision(&store,&current),lease,None,attempt_limits());
    assert!(matches!(result,Err(ProtocolError::Store(StoreError::Capacity(_)))),"capacity pressure erased the latest conclusive result");
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),before);let rows=store.attempt_summaries(current.id()).unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rows[0].outcome,Some(attempts::OutcomeKind::RefusedNoWrite));assert!(!rows[0].consumed);
    assert!(written.is_empty()&&current.request().native_progress().proven_zero());
}
#[test]
fn c8attempt_persisted_progress_and_chain_are_checked_against_actual_plan() {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new("attempt-corruption-source");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw(&"x".repeat(10000)).with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)))).unwrap();
    let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);let from=ex.envelope;assert!(matches!(agent_write::finish(job,&ex),Terminal::PartialExact{..}));
    let _next=attempt_prepare(&store,&current,Some(from));let original:serde_json::Value=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
    let mut accepted=Vec::new();
    for case in ["body-bound","unfinished-success","self-parent","parent-progress","missing-anchor","unconsumed-old","highwater-gap"] {
        let mut image=original.clone();let ledger=&mut image["rows"][0]["attempts"];
        // Test terminal projection independently of the live-parent cross-check.
        if matches!(case,"body-bound"|"unfinished-success") {
            ledger["entries"].as_object_mut().unwrap().remove("2");ledger["next_number"]=2.into();ledger["entries"]["1"]["consumed"]=false.into();
        }
        match case {
            "body-bound"=>{ledger["entries"]["1"]["outcome"]["envelope"]["body_offset"]=10001.into();ledger["entries"]["1"]["outcome"]["native"]["envelope"]["body_offset"]=10001.into();},
            "unfinished-success"=>{ledger["entries"]["1"]["outcome"]["kind"]="written_submitted".into();ledger["entries"]["1"]["outcome"]["envelope"]["submitted"]=true.into();ledger["entries"]["1"]["outcome"]["native"]["envelope"]["submitted"]=true.into();ledger["first_partial"]=serde_json::Value::Null;},
            "self-parent"=>ledger["entries"]["2"]["resume_parent"]=2.into(),
            "parent-progress"=>{ledger["entries"]["2"]["before"]["accepted_total"]=4095.into();ledger["entries"]["2"]["before"]["envelope"]["body_offset"]=4095.into();ledger["entries"]["2"]["before"]["run_accepted"]["payload"]=4095.into();ledger["entries"]["2"]["resume"]["body_offset"]=4095.into();},
            "missing-anchor"=>ledger["first_partial"]=serde_json::Value::Null,
            "unconsumed-old"=>ledger["entries"]["1"]["consumed"]=false.into(),
            "highwater-gap"=>ledger["next_number"]=4.into(),
            _=>unreachable!(),
        }
        let bad=Scratch::new(case);let bytes=serde_json::to_vec(&image).unwrap();write_private(&bad.0.join(DATA),&bytes);
        for _ in 0..2 {
            if !matches!(PendingStore::open_with_registry(&bad.0,11,attempt_store_limits(),registry_limits()),Err(StoreError::Invalid(_))) {accepted.push(case);}
            assert_eq!(fs::read(bad.0.join(DATA)).unwrap(),bytes);
        }
    }
    assert!(accepted.is_empty(),"persisted impossible progress accepted: {accepted:?}");
}

#[test]
fn c8attempt_projection_accepts_all_legal_plan_shapes_and_partial_phases() {
    use crate::agent_write::{self,PassThrough,Terminal};
    for plan in [Plan::raw(""),Plan::raw("body"),Plan::paste(""),Plan::paste("한글").with_clear(0).with_submit(0,0),Plan::control(vec![]),Plan::control(vec![27,91,65])] {
        let scratch=Scratch::new("attempt-plan-shape");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let expected=plan.planned_bytes();let submit=plan.owns_submit();let current=attempt_current(&store,&cap,plan);
        let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);
        let terminal=agent_write::finish(job,&ex);assert!(matches!(terminal,Terminal::WrittenBody{..}|Terminal::WrittenSubmitted{..}),"legal plan rejected: {terminal:?}");
        assert_eq!(matches!(terminal,Terminal::WrittenSubmitted{..}),submit);assert_eq!(written,expected);assert!(journal.has_stored_outcome());
    }
    struct PartialPhase(Vec<u8>);
    impl std::io::Write for PartialPhase {
        fn write(&mut self,b:&[u8])->io::Result<usize>{if self.0.is_empty(){self.0.push(b[0]);Ok(1)}else{Err(io::ErrorKind::BrokenPipe.into())}}
        fn flush(&mut self)->io::Result<()>{Ok(())}
    }
    for plan in [Plan::paste("body"),Plan::control(vec![27,91,65])] {
        let scratch=Scratch::new("attempt-partial-phase");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,plan);
        let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();let mut writer=PartialPhase(Vec::new());let ex=attempt_execute(&mut writer,&job);
        assert!(matches!(agent_write::finish(job,&ex),Terminal::PartialUnknown{..}));assert_eq!(writer.0.len(),1);assert!(journal.has_stored_outcome(),"partial fence/control lower bound must remain recordable");
        assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].native.unwrap().accepted_total,1);
    }
}

#[test]
fn c8attempt_another_requests_start_receipt_cannot_advance_this_journal() {
    use crate::agent_write::{self,PassThrough,Terminal};
    let scratch=Scratch::new("attempt-foreign-start");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();
    let a=attempt_current(&store,&cap,Plan::raw("same"));let b=attempt_current(&store,&cap,Plan::raw("same"));
    let pa=attempt_prepare(&store,&a,None);let pb=attempt_prepare(&store,&b,None);let mut job=pa.into_job(Arc::new(PassThrough)).unwrap();job.journal=Some(pb.journal());
    let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);assert!(matches!(ex.terminal,Terminal::RefusedNoWrite{..}));
    assert!(written.is_empty(),"another request's start authorized native bytes");
    assert_eq!(store.attempt_summaries(b.id()).unwrap()[0].phase,attempts::AttemptPhase::Prepared,"foreign start advanced journal");
    agent_write::finish(job,&ex);
}
#[test]
fn c8attempt_another_requests_zero_byte_terminal_cannot_close_this_journal() {
    use crate::agent_write::{self,PassThrough,Terminal};
    let scratch=Scratch::new("attempt-foreign-outcome");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();
    let a=attempt_current(&store,&cap,Plan::raw("same"));let b=attempt_current(&store,&cap,Plan::raw("same"));
    let pa=attempt_prepare(&store,&a,None);let pb=attempt_prepare(&store,&b,None);let journal=pb.journal();let mut job=pa.into_job(Arc::new(PassThrough)).unwrap();job.journal=Some(journal.clone());
    a.request().cancel();let mut written=Vec::new();let ex=attempt_execute(&mut written,&job);assert!(matches!(ex.terminal,Terminal::CancelledBeforeWrite));agent_write::finish(job,&ex);
    assert!(written.is_empty());assert_eq!(store.attempt_summaries(b.id()).unwrap()[0].phase,attempts::AttemptPhase::Prepared,"foreign zero-byte completion closed another attempt");
    assert!(!journal.has_stored_outcome());assert!(journal.retry_outcome().is_err(),"foreign receipt was retained as this journal's completion");
}

#[test]
fn c8attempt_resumed_writing_failure_can_record_unchanged_prior_prefix() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError,Accepted};
    let scratch=Scratch::new("attempt-resumed-io-refusal");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("attempt")).unwrap();let current=attempt_current(&store,&cap,Plan::raw(&"x".repeat(10000)).with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)))).unwrap();
    let mut written=Vec::new();let first=attempt_execute(&mut written,&job);let prefix=first.envelope;assert!(matches!(agent_write::finish(job,&first),Terminal::PartialExact{..}));let before=written.clone();
    let p=attempt_prepare(&store,&current,Some(prefix));let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();disk.fail.store(true,Ordering::Release);
    let ex=attempt_execute(&mut written,&job);assert!(matches!(ex.terminal,Terminal::PartialExact { accepted:Accepted { payload:0,fence:0,control:0,calls:0 },.. }));
    assert!(matches!(agent_write::finish(job,&ex),Terminal::PartialUnknown{..}));assert_eq!(written,before);
    disk.fail.store(false,Ordering::Release);let result=journal.retry_outcome();
    assert!(result.is_ok(),"actual unchanged resumed prefix could not be durably recorded: {result:?}");
    let rows=store.attempt_summaries(current.id()).unwrap();assert_eq!(rows.last().unwrap().outcome,Some(attempts::OutcomeKind::PartialExact));
    assert_eq!(rows.last().unwrap().accepted,Some(Accepted::default()));assert_eq!(rows.last().unwrap().native.unwrap().envelope,prefix);assert_eq!(written,before);
    assert!(matches!(current.request().reserve_attempt(current.plan(),Some(prefix)),Err(LeaseError::Busy)),"outcome retry must not release native execution authority");
}

#[test]
fn c8reconcile_stored_zero_outcome_releases_only_for_same_request_retry() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("reconcile-zero");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("original").with_submit(0,0));let id=current.id().to_owned();let created=current.received_at();
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();disk.fail.store(true,Ordering::Release);let mut bytes=Vec::new();
    let ex=attempt_execute(&mut bytes,&job);assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    assert!(journal.resolve_stored_zero_write().is_err());assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)));
    disk.fail.store(false,Ordering::Release);journal.retry_outcome().unwrap();
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),"storage retry alone released native ownership");
    let result=journal.resolve_stored_zero_write();assert!(result.is_ok(),"actual stored zero result failed to release its own reservation: {result:?}");
    let p=attempt_prepare(&store,&current,None);assert_eq!(p.number(),2);let job=p.into_job(Arc::new(PassThrough)).unwrap();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(bytes,b"original\r");assert_eq!(current.id(),id);assert_eq!(current.received_at(),created);assert_eq!(store.inventory().unwrap().len(),1);
}
struct NoDispatchProof;
impl crate::agent_write::Proof for NoDispatchProof {
    fn admit(&self)->Result<(),crate::receiver::ProofLoss>{panic!("undispatched refusal must not collect native proof")}
    fn recheck(&self,_:crate::agent_write::Phase)->Result<(),crate::receiver::ProofLoss>{self.admit()}
}
#[test]
fn c8reconcile_declined_prepared_attempt_is_recorded_without_native_io() {
    use crate::agent_write::{self,PassThrough,Terminal};
    let scratch=Scratch::new("reconcile-declined");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("original").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();let terminal=agent_write::decline_unstarted(job,"queue_full");
    assert!(matches!(terminal,Terminal::RefusedNoWrite{..}));assert!(current.request().native_progress().proven_zero());
    assert_eq!(store.attempt_summaries(current.id()).unwrap()[0].phase,attempts::AttemptPhase::Outcome,"unstarted decline left a durable Prepared orphan");
    assert!(journal.has_stored_outcome());let data:serde_json::Value=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
    assert!(data["rows"][0]["attempts"]["entries"]["1"]["writing"].is_null());assert_eq!(data["rows"][0]["attempts"]["entries"]["1"]["outcome"]["accepted"]["calls"],0);
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(PassThrough)).unwrap();let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(bytes,b"original\r");
}
#[test]
fn c8reconcile_declined_resume_preserves_prefix_and_first_loss_time() {
    use crate::agent_write::{self,PassThrough,Terminal};
    let scratch=Scratch::new("reconcile-declined-resume");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let body="x".repeat(10000);let current=attempt_current(&store,&cap,Plan::raw(&body).with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)))).unwrap();let mut bytes=Vec::new();let first=attempt_execute(&mut bytes,&job);let prefix=first.envelope;
    assert!(matches!(agent_write::finish(job,&first),Terminal::PartialExact{..}));let lost=current.request().lost_at();assert!(lost.is_some());let before=bytes.clone();
    let p=attempt_prepare(&store,&current,Some(prefix));let journal=p.journal();let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();let terminal=agent_write::decline_unstarted(job,"queue_full");
    assert!(matches!(terminal,Terminal::PartialExact{..}),"decline did not retain the actual pre-existing exact prefix: {terminal:?}");
    assert!(journal.has_stored_outcome());assert_eq!(current.request().lost_at(),lost);assert_eq!(bytes,before);assert_eq!(current.request().native_progress().run_accepted,Default::default());
    let p=attempt_prepare(&store,&current,Some(prefix));let job=p.into_job(Arc::new(PassThrough)).unwrap();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(bytes,[body.as_bytes(),b"\r"].concat());
}

fn reconcile_failed_zero_finish(store:&Arc<PendingStore>,current:&Arc<CurrentRequest>,disk:&Arc<SwitchDisk>)->Arc<attempts::AttemptJournal> {
    use crate::agent_write::{self,Terminal};
    let p=attempt_prepare(store,current,None);let journal=p.journal();let job=p.into_job(Arc::new(AttemptRefusal)).unwrap();let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    disk.fail.store(true,Ordering::Release);assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));assert!(bytes.is_empty());
    disk.fail.store(false,Ordering::Release);journal.retry_outcome().unwrap();journal
}
#[test]
fn c8reconcile_old_receipt_cannot_release_a_new_started_attempt_or_another_request() {
    use crate::agent_write::LeaseError;
    let scratch=Scratch::new("reconcile-stale");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let a=attempt_current(&store,&cap,Plan::raw("same"));let b=attempt_current(&store,&cap,Plan::raw("same"));
    let first=reconcile_failed_zero_finish(&store,&a,&disk);let wrong=first.zero_write_receipt().unwrap();
    assert!(b.request().resolve_durable_zero_write(wrong).is_err());
    let stale=first.zero_write_receipt().unwrap();first.resolve_stored_zero_write().unwrap();let second=reconcile_failed_zero_finish(&store,&a,&disk);
    assert!(a.request().resolve_durable_zero_write(stale).is_err(),"stale receipt released another Started attempt");
    assert!(matches!(a.request().reserve_attempt(a.plan(),None),Err(LeaseError::Busy)));second.resolve_stored_zero_write().unwrap();
    let third=attempt_prepare(&store,&a,None);assert_eq!(third.number(),3);
    assert!(first.resolve_stored_zero_write().is_err());
    assert!(matches!(a.request().reserve_attempt(a.plan(),None),Err(LeaseError::Busy)),"old journal released a newer Prepared attempt");
}
#[test]
fn c8reconcile_cancelled_zero_resolution_preserves_cancellation() {
    use crate::agent_write::{self,Terminal,LeaseError};
    let scratch=Scratch::new("reconcile-cancelled-zero");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("original"));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();current.request().cancel();disk.fail.store(true,Ordering::Release);
    assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::CancelledBeforeWrite));disk.fail.store(false,Ordering::Release);journal.retry_outcome().unwrap();journal.resolve_stored_zero_write().unwrap();
    assert!(current.request().is_cancelled());assert!(matches!(current.request().status(),agent_write::Status::Terminal(Terminal::CancelledBeforeWrite)));
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Cancelled)),"native cleanup revived a cancelled request");
}
#[test]
fn c8reconcile_cancelled_resume_decline_is_not_advertised_as_resumable() {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new("reconcile-cancelled-resume");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let current=attempt_current(&store,&cap,Plan::raw(&"x".repeat(10000)).with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)))).unwrap();let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);let prefix=ex.envelope;
    assert!(matches!(agent_write::finish(job,&ex),Terminal::PartialExact{..}));let before=bytes.clone();let p=attempt_prepare(&store,&current,Some(prefix));let journal=p.journal();let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();current.request().cancel();
    let terminal=agent_write::decline_unstarted(job,"queue_full");assert!(matches!(terminal,Terminal::PartialUnknown{..}),"cancelled partial was advertised as resumable: {terminal:?}");
    assert!(journal.has_stored_outcome());assert!(journal.resolve_stored_zero_write().is_err());assert_eq!(bytes,before);
}
#[test]
fn c8reconcile_success_partial_and_uncertain_receipts_never_mint_zero_release() {
    use crate::agent_write::{self,PassThrough,Terminal};
    struct Panics;
    impl std::io::Write for Panics {fn write(&mut self,_:&[u8])->io::Result<usize>{panic!("controlled native write panic")}fn flush(&mut self)->io::Result<()>{Ok(())}}
    for case in ["empty-success","partial","unknown"] {
        let scratch=Scratch::new(case);let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let plan=if case=="empty-success"{Plan::raw("")}else{Plan::raw(&"x".repeat(10000))};let current=attempt_current(&store,&cap,plan);
        let p=attempt_prepare(&store,&current,None);let journal=p.journal();let proof:Arc<dyn agent_write::Proof>=if case=="partial"{Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)))}else{Arc::new(PassThrough)};let job=p.into_job(proof).unwrap();
        let ex=if case=="unknown" {attempt_execute(&mut Panics,&job)}else{attempt_execute(&mut Vec::new(),&job)};let terminal=agent_write::finish(job,&ex);
        match case {"empty-success"=>{assert!(matches!(terminal,Terminal::WrittenBody{..}));assert!(current.request().native_progress().proven_zero());},"partial"=>assert!(matches!(terminal,Terminal::PartialExact{..})),_=>assert!(matches!(terminal,Terminal::PartialUnknown{..}))}
        assert!(journal.has_stored_outcome());assert!(journal.zero_write_receipt().is_err(),"non-refusal outcome minted zero-release authority: {case}");
    }
}
#[test]
fn c8reconcile_zero_release_waits_for_actual_finish_to_leave_its_settlement() {
    use crate::agent_write::{self,Journal,Terminal};
    struct ErrorAfterStore(Arc<attempts::AttemptJournal>);
    impl Journal for ErrorAfterStore {
        fn writing(&self,r:&agent_write::Request)->io::Result<()>{self.0.writing(r)}
        fn terminal(&self,r:&agent_write::Request,t:&Terminal,e:&agent_write::Envelope)->io::Result<()>{self.0.terminal(r,t,e)}
        fn writing_attempt(&self,c:&agent_write::ProofContext<'_>,r:Option<&agent_write::AttemptStartReceipt<'_>>)->io::Result<()>{self.0.writing_attempt(c,r)}
        fn terminal_receipt(&self,r:&agent_write::WriterReceipt<'_>)->io::Result<()>{self.0.terminal_receipt(r)?;Err(io::Error::other("after actual store success"))}
    }
    let scratch=Scratch::new("reconcile-settling");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned"));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let mut job=p.into_job(Arc::new(AttemptRefusal)).unwrap();job.journal=Some(Arc::new(ErrorAfterStore(journal.clone())));
    let rejected=Arc::new(AtomicBool::new(false));let result=rejected.clone();let inside=journal.clone();job.settle=Some(Box::new(move |_|result.store(inside.resolve_stored_zero_write().is_err(),Ordering::Release)));
    let ex=attempt_execute(&mut Vec::new(),&job);assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    assert!(rejected.load(Ordering::Acquire),"release overtook actual finish/settlement ownership");journal.resolve_stored_zero_write().unwrap();assert!(current.request().reserve_attempt(current.plan(),None).is_ok());
}
#[test]
fn c8reconcile_decline_wrong_job_has_no_outcome_or_settlement_authority() {
    use crate::agent_write::{self,Terminal};
    let scratch=Scratch::new("reconcile-wrong-decline");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let a=attempt_current(&store,&cap,Plan::raw("same"));let b=attempt_current(&store,&cap,Plan::raw("same"));
    let p=attempt_prepare(&store,&a,None);let journal=p.journal();let mut job=p.into_job(Arc::new(NoDispatchProof)).unwrap();job.request=b.request().clone();
    let settled=Arc::new(AtomicBool::new(false));let flag=settled.clone();job.settle=Some(Box::new(move |_|flag.store(true,Ordering::Release)));
    assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::PartialUnknown{..}));
    assert!(!settled.load(Ordering::Acquire)&&!journal.has_stored_outcome(),"wrong decline closed another request");
    assert_eq!(store.attempt_summaries(a.id()).unwrap()[0].phase,attempts::AttemptPhase::Prepared);
}

#[test]
fn c8reconcile_repeat_ack_is_idempotent_only_before_new_ownership() {
    use crate::agent_write::{self,Terminal};
    for next in ["attempt","edit"] {
        let scratch=Scratch::new("reconcile-repeat-ack");let disk=Arc::new(SwitchDisk { fail:AtomicBool::new(false) });
        let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("reconcile")).unwrap();let current=attempt_current(&store,&cap,Plan::raw("owned"));
        let journal=reconcile_failed_zero_finish(&store,&current,&disk);let stale=journal.zero_write_receipt().unwrap();journal.resolve_stored_zero_write().unwrap();
        assert!(journal.resolve_stored_zero_write().is_ok(),"same completed resolution cannot acknowledge an unchanged retry");
        if next=="attempt" {
            let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();
            assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::RefusedNoWrite{..}));
        } else { drop(current.request().reserve_zero_write_edit().unwrap()); }
        assert!(current.request().native_progress().proven_zero());
        assert!(current.request().resolve_durable_zero_write(stale).is_err(),"old acknowledgement survived new {next} ownership");
    }
}

}
