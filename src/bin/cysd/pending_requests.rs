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

const SCHEMA: u32 = 3;
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
    pub epochs: EpochStamp,
    pub policy_digest: String,
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
            .field("epochs", &self.epochs)
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
            target:self.target, epochs:self.epochs, policy_digest:self.policy_digest,
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
struct Row { record:PendingRecord, terminal:Option<TerminalRecord>, settled:Option<Settled> }
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
    pub target:Target, pub epochs:EpochStamp, pub body_bytes:usize,
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
        Ok(Self { root,instance,limits, mutation:Mutex::new(State { rows,loaded,registry,live_registrations:BTreeMap::new(),loaded_registrations,
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
            if record.epochs.daemon_instance!=scope.daemon_instance || record.epochs.surface_instance!=scope.surface_instance
                || record.epochs.launch!=scope.launch || record.epochs.policy!=scope.policy
                || record.target.surface_id!=scope.surface_id { return Err(StoreError::Conflict); }
            record.registration_id=Some(registration.id().to_owned());
        }
        validate_record(&record)?;
        check_raw_record_size(&record,self.limits.max_record_bytes)?;
        let row=Row { record,terminal:None,settled:None };
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
                source:r.source.clone(),target:r.target.clone(),epochs:r.epochs,
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
    if r.obligation_id.is_empty() || r.policy_digest.is_empty() || r.original_body_digest.is_empty()
        || r.epochs.daemon_instance == 0 || r.epochs.surface_instance == 0 {
        return Err(StoreError::Invalid("provenance"));
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
        r.request_id.len(), r.obligation_id.len(), r.policy_digest.len(),
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
    let actual=encode_limited(row,l.max_record_bytes)?.len();
    if row.settled.is_some() { return Ok(actual); }
    #[derive(Serialize)]
    struct Bare<'a> { record:&'a PendingRecord, terminal:Option<()>, settled:Option<()> }
    let bare=encode_limited(&Bare { record:&row.record,terminal:None,settled:None },l.max_record_bytes)?.len();
    let charged=bare.checked_sub(8).and_then(|n|n.checked_add(l.max_terminal_bytes))
        .and_then(|n|n.checked_add(l.max_settlement_bytes)).ok_or(StoreError::Capacity("row reserve overflow"))?;
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
        Some(3)=>serde_json::from_value::<Snapshot>(value).map_err(|_|StoreError::Invalid("schema 3 shape"))?,
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
                source:r.source.clone(),target:r.target.clone(),epochs:r.epochs,
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
            let s=registered.scope;let r=&row.record;
            if r.epochs.daemon_instance!=s.daemon_instance || r.epochs.surface_instance!=s.surface_instance
                || r.epochs.launch!=s.launch || r.epochs.policy!=s.policy || r.target.surface_id!=s.surface_id {
                return Err(StoreError::Invalid("request registration scope"));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool,AtomicU64,Ordering};
    use std::sync::mpsc;
    use std::time::Duration;
    static NEXT_SCRATCH:AtomicU64=AtomicU64::new(1);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name:&str)->Self {
            let base=PathBuf::from(std::env::var_os("C8_PENDING_TEST_ROOT")
                .expect("set explicit isolated C8_PENDING_TEST_ROOT")).canonicalize().unwrap();
            let path=base.join(format!("reclaim-{name}-{}-{}",std::process::id(),NEXT_SCRATCH.fetch_add(1,Ordering::Relaxed)));
            let mut builder=fs::DirBuilder::new();
            #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
            builder.create(&path).unwrap();
            assert!(path.canonicalize().unwrap().starts_with(&base));
            Self(path)
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
        let row=Row { record:spec("same").into_record("wb-1".into()),terminal:None,settled:None };
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
            let scratch=Scratch::new(mode);let row=Row { record:spec("x").into_record("wb-3".into()),terminal:None,settled:None };
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
                let row=Row { record:spec("loaded").into_record("wb-1".into()),
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
}
