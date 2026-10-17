//! Native receiver publication state; input, registration and durability adapters remain explicit.
#![allow(dead_code)]

use crate::agent_write::Request;
use crate::receiver::{self, AttemptTicket, Binding, DiscoveryInput, Epochs, Executor,
    Node, ProofClass, ProofLoss, PublicationGate,
    PublicationPhase, ReceiverPolicy, RegistrationOrigin, StartRefusal, Unknown};
use crate::state::Surface;
use crate::writer_gate::WriterGate;
use cys::process_observation::{self, Failure, ProcessObservation, Retained};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool,Ordering};
use std::sync::{Arc,Mutex,MutexGuard,TryLockError,Weak};
use std::time::{Duration,Instant};

type NativeId=(u32,u64,u64);
fn native_id(o:&ProcessObservation)->NativeId {
    (o.identity.pid,o.identity.birth_seconds,o.identity.birth_microseconds)
}
fn same_contract(a:&ReceiverPolicy,b:&ReceiverPolicy)->bool {
    a.adapter==b.adapter && a.declaration==b.declaration && a.config_digest==b.config_digest
        && a.recipe==b.recipe && a.class==b.class && a.weak_risk==b.weak_risk
}
fn increment(v:u64)->Result<u64,CoreError> { v.checked_add(1).ok_or(CoreError::EpochExhausted) }

#[derive(Clone,Debug,PartialEq,Eq,Serialize)]
pub(crate) enum Target { Surface, Role(String) }
pub(crate) struct RegistrationObligation {
    id:String,surface_id:u64,cancelled:AtomicBool,
}
impl RegistrationObligation {
    pub fn new(id:String,surface_id:u64)->Arc<Self> {
        Arc::new(Self { id,surface_id,cancelled:AtomicBool::new(false) })
    }
}
#[derive(Clone)]
pub(crate) enum DiscoveryOwner {
    Registration(Arc<RegistrationObligation>),
    ExistingWrite(Arc<Request>),
}
#[derive(Clone)]
pub(crate) struct Work {
    pub owner:DiscoveryOwner,pub obligation_id:String,pub target:Target,pub expected:Epochs,
}
impl Work {
    fn same(&self,other:&Work)->bool {
        let same_owner=match (&self.owner,&other.owner) {
            (DiscoveryOwner::Registration(a),DiscoveryOwner::Registration(b))=>Arc::ptr_eq(a,b),
            (DiscoveryOwner::ExistingWrite(a),DiscoveryOwner::ExistingWrite(b))=>Arc::ptr_eq(a,b),
            _=>false,
        };
        same_owner && self.obligation_id==other.obligation_id && self.target==other.target && self.expected==other.expected
    }
    fn id(&self)->&str { match &self.owner { DiscoveryOwner::Registration(r)=>&r.id,DiscoveryOwner::ExistingWrite(r)=>&r.id } }
    fn ticket_id(&self)->String { match &self.owner {
        DiscoveryOwner::Registration(r)=>format!("registration:{}",r.id),DiscoveryOwner::ExistingWrite(r)=>r.id.clone(),
    } }
    fn surface_id(&self)->u64 { match &self.owner { DiscoveryOwner::Registration(r)=>r.surface_id,DiscoveryOwner::ExistingWrite(r)=>r.surface_id } }
    fn cancelled(&self)->bool { match &self.owner { DiscoveryOwner::Registration(r)=>r.cancelled.load(Ordering::Acquire),DiscoveryOwner::ExistingWrite(r)=>r.is_cancelled() } }
    fn cancel(&self) { match &self.owner { DiscoveryOwner::Registration(r)=>r.cancelled.store(true,Ordering::Release),DiscoveryOwner::ExistingWrite(r)=>r.cancel() } }
    fn proven_zero(&self)->bool { match &self.owner {
        DiscoveryOwner::Registration(_)=>true, // typed no-write obligation, never an AgentJob/Request
        DiscoveryOwner::ExistingWrite(r)=>r.native_progress().proven_zero(),
    } }
    fn request_id(&self)->Option<String> { match &self.owner { DiscoveryOwner::Registration(_)=>None,DiscoveryOwner::ExistingWrite(r)=>Some(r.id.clone()) } }
    fn registration_id(&self)->Option<String> { match &self.owner { DiscoveryOwner::Registration(r)=>Some(r.id.clone()),DiscoveryOwner::ExistingWrite(_)=>None } }
}

/// A native binding with an irreversible seal. No mutable Binding accessor exists.
/// check_native is ONLY the native component, never an agent-write admission proof.
pub(crate) struct PublishedNative { binding:Arc<Binding>, sealed:AtomicBool, active:AtomicBool }
impl PublishedNative {
    fn new(binding:Arc<Binding>)->Self { Self { binding,sealed:AtomicBool::new(false),active:AtomicBool::new(false) } }
    pub fn epochs(&self)->Epochs { self.binding.epochs }
    pub fn agent_id(&self)->NativeId { self.binding.agent_identity() }
    pub fn sealed(&self)->bool { self.sealed.load(Ordering::Acquire) }
    fn seal(&self) { self.sealed.store(true,Ordering::Release); }
    pub fn check_native(&self,foreground:&dyn Fn()->Option<u32>)->Result<(),ProofLoss> {
        if self.sealed() || !self.active.load(Ordering::Acquire) { return Err(ProofLoss::EpochMoved); }
        self.binding.check(foreground)?;
        if self.sealed() || !self.active.load(Ordering::Acquire) { return Err(ProofLoss::EpochMoved); }
        Ok(())
    }
}

/// Private native fact: metadata, root death, Changed and Unknown cannot construct it.
pub(crate) struct AgentDead {
    old:Arc<PublishedNative>, base:Epochs, identity:NativeId, observed_at:Instant,
}
#[derive(Debug)]
pub(crate) enum DeathError { NoCurrent, Alive, Native(Failure), Superseded, Busy }

// Private result classification; production supplies only the retained agent query above.
// The test seam injects a FAILURE, not a fabricated successful native observation.
fn finish_agent_death_probe(old:Arc<PublishedNative>,base:Epochs,result:Result<(),Failure>)
    ->Result<AgentDead,DeathError>
{
    match result {
        Err(Failure::Dead)=>Ok(AgentDead { identity:old.agent_id(),old,base,observed_at:Instant::now() }),
        Ok(())=>Err(DeathError::Alive),
        Err(other)=>Err(DeathError::Native(other)),
    }
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Mode { Initial, Adopt, Change }
#[derive(Clone)]
enum PolicySlot { Catalog, Fixed(Arc<ReceiverPolicy>) }
#[derive(Clone,Debug)]
pub(crate) enum Registration {
    Selected, SameUnbound, SamePublishedNative, SameSealedNative,
    NeedsAuthorizedDifferentBuiltin, NeedsAuthorizedConfigurationChange,
}
#[derive(Clone,Debug)]
pub(crate) enum CoreError {
    Busy, Superseded, NoPending, PendingConflict, NoCurrent, NotDead, AlreadyBound,
    Cancelled, Expired, WindowNotOpened, PossibleWrite, WriterBusy, EpochExhausted,
    InvalidPolicy, UnsupportedTransition, Authority(String), Input(String), Native(Unknown),
    WeakServiceRequired(ReceiverPolicy), WeakCatalogServiceRequired(Vec<ReceiverPolicy>), Publish(receiver::PublishRefusal), Start(StartRefusal),
}

/// Integration facts are captured outside the receiver mutex. These fields are
/// compare inputs, NOT proof that the screen is safe merely because they exist.
#[derive(Clone,Debug,PartialEq,Eq)]
pub(crate) struct InputStamp {
    pub epochs:Epochs,
    pub surface_id:u64,
    pub role:Option<String>,
    pub input_generation:u64,
    pub output_generation:u64,
    pub raw_human_bytes:u64,
    pub pending_bytes:u64,
    pub last_human_input:Option<Instant>,
}
impl InputStamp {
    /// A repaint is not an authority change. The validator must freshly verify
    /// each sampled frame, but persistence does not require an output ACK/quiet
    /// window or the same output generation across its two validated samples.
    fn same_authority(&self,other:&Self)->bool {
        self.epochs==other.epochs && self.surface_id==other.surface_id && self.role==other.role
            && self.input_generation==other.input_generation && self.raw_human_bytes==other.raw_human_bytes
            && self.pending_bytes==other.pending_bytes && self.last_human_input==other.last_human_input
    }
}
pub(crate) struct CommitContext<'a> {
    pub work:&'a Work, pub policy:&'a ReceiverPolicy, pub prospective:Epochs,
    pub root:NativeId, pub agent:NativeId, pub native_validated_at:Instant,
}
/// REQUIRED integration seam, intentionally no production implementation here.
/// acquire_and_validate must capture actual current transaction-aware input state,
/// reject human/foreign draft, modal/approval/bare-shell/unrecognized state, verify
/// the frozen adapter predicate and exact role mapping, and obtain a guard that
/// KEEPS those comparison inputs serialized until Drop. It must use nonblocking
/// acquisition and the declared writer-reservation -> input/role barrier -> cell
/// order. Never call parser/native/config I/O while that barrier is held.
/// The root must provide this adapter; arbitrary booleans are not accepted.
pub(crate) trait PublicationValidator {
    type Guard<'a> where Self:'a;
    fn acquire_and_validate<'a>(&'a self,context:&CommitContext<'_>)
        ->Result<(Self::Guard<'a>,InputStamp),String>;
}

#[derive(Clone,Debug)]
pub(crate) struct ChangeContext {
    pub surface_id:u64,pub base:Epochs, pub old_adapter:String, pub old_config:String,
    pub new_adapter:String, pub new_config:String, pub origin:RegistrationOrigin,
}
/// REQUIRED authenticated claim_role/reclaim_commit adapter. The implementation
/// must verify the actual successful transaction and its scope, not a wire flag.
pub(crate) trait RegistrationAuthorizer {
    fn validate_registration(&self,context:&ChangeContext)->Result<String,String>;
}
pub(crate) struct ChangeGrant {
    old:Arc<PublishedNative>, base:Epochs, proposed:Arc<ReceiverPolicy>,
    origin:RegistrationOrigin, evidence_ref:String,
}

/// Native input is constructed only from this seat in production. Its closures
/// never run under the receiver mutex. Tests may create private fixture SeatIo.
struct SeatIo {
    surface_id:u64, root_pid:u32,
    foreground:Arc<dyn Fn()->Option<u32>+Send+Sync>, writer:WriterGate,
}
impl SeatIo {
    fn from_surface(surface:&Arc<Surface>)->Self {
        let weak=Arc::downgrade(surface);
        let foreground:Arc<dyn Fn()->Option<u32>+Send+Sync>=Arc::new(move||{
            let s=weak.upgrade()?;
            #[cfg(unix)] { s.foreground_pgid_now().map(|n|n as u32) }
            #[cfg(not(unix))] { let _=s;None }
        });
        // No native call: first ACTUAL started collection obtains the root birth anchor.
        Self { surface_id:surface.id,root_pid:surface.pid,
            foreground,writer:surface.write_tx.gate() }
    }
    fn discovery(&self,policy:&ReceiverPolicy,epochs:Epochs,sealed:Vec<NativeId>)->DiscoveryInput {
        let foreground=self.foreground.clone();
        DiscoveryInput { root_pid:self.root_pid,recipe:policy.recipe.clone(),class:policy.class,
            epochs,sealed,snapshot:Box::new(real_topology),foreground:Box::new(move||foreground()),
            observe:Box::new(process_observation::observe),retain:Box::new(Retained::open) }
    }
}
fn real_topology()->Vec<Node> {
    let mut system=sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All,true);
    system.processes().iter().filter_map(|(pid,p)|p.parent().map(|parent|Node {
        pid:pid.as_u32(),parent:parent.as_u32() })).collect()
}

struct Pending {
    episode:u64,work:Work, mode:Mode, prospective:Epochs, selected:Option<Arc<ReceiverPolicy>>,
    old:Option<Arc<PublishedNative>>, authorization:Option<String>,
    gate:PublicationGate, active_attempt:Option<u64>, staged_attempt:Option<u64>, first_windows:Option<WindowsSample>,
}
struct Core {
    epochs:Epochs, root:Option<NativeId>,catalog:Arc<[ReceiverPolicy]>, policy:PolicySlot,
    current:Option<Arc<PublishedNative>>, dead:HashSet<NativeId>,
    pending:Option<Pending>,next_attempt:u64,next_episode:u64,
}
pub(crate) struct ReceiverCell { io:SeatIo, executor:Arc<Executor>,windows:bool, core:Mutex<Core> }

/// A returned candidate can only originate in this module's started discovery.
/// It is non-Clone. Native handles and its completion lease are owned OUTSIDE Core.
pub(crate) struct NativeCandidate {
    lease:ActualAttempt, policy:Arc<ReceiverPolicy>, binding:Arc<Binding>,
    summary:Arc<NativeSummary>,validated_at:Instant,
}
struct NativeSummary {
    root:ProcessObservation,agent:ProcessObservation,ancestors:Vec<ProcessObservation>,
}
impl PartialEq for NativeSummary {
    fn eq(&self,o:&Self)->bool { self.root==o.root && self.agent==o.agent && self.ancestors==o.ancestors }
}
struct WindowsSample {
    base:Epochs,old:Arc<PublishedNative>,contract:Arc<ReceiverPolicy>,ticket:AttemptTicket,
    started_at:Instant,validated_at:Instant,summary:Arc<NativeSummary>,
}
struct ActualAttempt {
    cell:Weak<ReceiverCell>,ticket:AttemptTicket,episode:u64,started_at:Instant,
}
impl Drop for ActualAttempt {
    fn drop(&mut self) {
        if let Some(cell)=self.cell.upgrade() {
            let mut core=cell.core.lock().unwrap_or_else(|e|e.into_inner());
            if let Some(p)=core.pending.as_mut() {
                if p.episode==self.episode && p.gate.ticket==self.ticket && p.active_attempt==Some(self.ticket.attempt_id) {
                    p.active_attempt=None;
                    p.staged_attempt=None;
                }
            }
        }
    }
}
struct Prepared {
    lease:ActualAttempt,mode:Mode,catalog:Arc<[ReceiverPolicy]>,selected:Option<Arc<ReceiverPolicy>>,
    prospective:Epochs,sealed:Vec<NativeId>,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize)]
pub(crate) struct TransitionRecord {
    pub request_id:Option<String>,pub registration_id:Option<String>,pub obligation_id:String,pub target:Target,
    pub old_epochs:Epochs,pub new_epochs:Epochs,
    pub old_agent:Option<NativeId>,pub new_agent:NativeId,pub root:NativeId,
    pub adapter:String,pub config_digest:String,pub class:ProofClass,
    pub origin:RegistrationOrigin,pub registration_evidence:Option<String>,
    pub episode:u64,pub attempt_id:u64,pub input_generation:u64,pub resolved_role:Option<String>,
}
pub(crate) enum PreparedOutcome {
    WindowsFirstSample,
    Prepared(PreparedPublication),
}
// No public token accessor: other request/proof constructors cannot obtain an
// executable token from staging. A failed/dropped stage never activates it.
pub(crate) struct PreparedPublication {
    candidate:NativeCandidate,token:Arc<PublishedNative>,record:TransitionRecord,stamp:InputStamp,
}
impl PreparedPublication {
    pub fn record(&self)->&TransitionRecord { &self.record }
    pub fn durable_capability(&self,store:&impl DurableActivationValidator)->Result<DurableActivation,CoreError> {
        let reference=store.validate_transition_and_retokenization(&self.record).map_err(CoreError::Authority)?;
        if reference.is_empty() { return Err(CoreError::Authority("missing durable store reference".into())); }
        Ok(DurableActivation { record:self.record.clone(),token:self.token.clone(),reference })
    }
}
/// REQUIRED store integration: verify a durable matching transition and only
/// proven-zero eligible request retokenization. Returning Ok is not supplied by
/// this draft. Do not treat a prepared WAL row as an executed write or a published
/// binding; ambiguous/recovered records never gain this in-process capability.
pub(crate) trait DurableActivationValidator {
    fn validate_transition_and_retokenization(&self,record:&TransitionRecord)->Result<String,String>;
}
pub(crate) struct DurableActivation { record:TransitionRecord,token:Arc<PublishedNative>,reference:String }
pub(crate) struct ActivatedPublication {
    pub token:Arc<PublishedNative>,pub record:TransitionRecord,pub durable_reference:String,
}

impl ReceiverCell {
    /// The declaration catalog is supplied by the trusted configured/embedded
    /// policy loader; this core never derives proof class from discovery success.
    pub fn new_native(surface:&Arc<Surface>,executor:Arc<Executor>,mut epochs:Epochs,mut catalog:Vec<ReceiverPolicy>,
        known_adapter:Option<&str>)->Result<Arc<Self>,CoreError>
    {
        if epochs.daemon_instance==0 || epochs.surface_instance==0 || catalog.is_empty() {
            return Err(CoreError::InvalidPolicy);
        }
        if epochs.policy==0 { epochs.policy=1; }
        let mut names=HashSet::new();
        for p in &mut catalog {
            if p.adapter.is_empty() || p.declaration.is_empty() || p.config_digest.is_empty()
                || p.recipe.bin.is_empty() || !names.insert(p.adapter.clone())
                || (p.class==ProofClass::FullRequired && p.weak_risk.is_some())
                || (p.class==ProofClass::ObservedAgentWeak && p.weak_risk.is_none()) {
                return Err(CoreError::InvalidPolicy);
            }
            p.policy_epoch=epochs.policy;
        }
        if !receiver::native_backend_available() { return Err(CoreError::WeakCatalogServiceRequired(catalog)); }
        let policy=match known_adapter {
            Some(name)=>{
                let p=catalog.iter().find(|p|p.adapter==name).ok_or(CoreError::InvalidPolicy)?.clone();
                if p.class==ProofClass::ObservedAgentWeak { return Err(CoreError::WeakServiceRequired(p)); }
                PolicySlot::Fixed(Arc::new(p))
            }
            None=>PolicySlot::Catalog,
        };
        // A truly unsupported backend belongs to the separate weak service. This
        // native constructor is not the population router and does not change class.
        let io=SeatIo::from_surface(surface);
        Ok(Arc::new(Self { io,executor,windows:cfg!(windows),core:Mutex::new(Core {
            epochs,root:None,catalog:catalog.into(),policy,current:None,dead:HashSet::new(),pending:None,next_attempt:1,next_episode:1 }) }))
    }
    fn try_core(&self)->Result<MutexGuard<'_,Core>,CoreError> {
        match self.core.try_lock() {
            Ok(g)=>Ok(g),Err(TryLockError::Poisoned(e))=>Ok(e.into_inner()),
            Err(TryLockError::WouldBlock)=>Err(CoreError::Busy),
        }
    }
    pub fn epochs(&self)->Epochs { self.core.lock().unwrap_or_else(|e|e.into_inner()).epochs }
    pub fn current(&self)->Option<Arc<PublishedNative>> {
        self.core.lock().unwrap_or_else(|e|e.into_inner()).current.clone()
    }
    pub fn fixed_policy(&self)->Option<Arc<ReceiverPolicy>> {
        match &self.core.lock().unwrap_or_else(|e|e.into_inner()).policy {
            PolicySlot::Catalog=>None,PolicySlot::Fixed(p)=>Some(p.clone()),
        }
    }
    /// Same contract is a no-op, including origin/policy epoch/window/start budget.
    /// A different adapter only reports the required route; it changes no policy.
    pub fn register_known(&self,proposed:&ReceiverPolicy)->Result<Registration,CoreError> {
        let mut core=self.try_core()?;
        match &core.policy {
            PolicySlot::Fixed(old) if same_contract(old,proposed)=>{
                return Ok(match &core.current {
                    None=>Registration::SameUnbound,
                    Some(t) if t.sealed()=>Registration::SameSealedNative,
                    Some(_)=>Registration::SamePublishedNative,
                });
            }
            PolicySlot::Fixed(old)=>return Ok(if old.adapter!=proposed.adapter
                && receiver::BUILTIN_ADAPTERS.contains(&old.adapter.as_str())
                && receiver::BUILTIN_ADAPTERS.contains(&proposed.adapter.as_str()) {
                    Registration::NeedsAuthorizedDifferentBuiltin
                } else { Registration::NeedsAuthorizedConfigurationChange }),
            PolicySlot::Catalog=>{},
        }
        let chosen=core.catalog.iter().find(|p|same_contract(p,proposed)).ok_or(CoreError::InvalidPolicy)?.clone();
        let chosen=Arc::new(chosen);
        core.policy=PolicySlot::Fixed(chosen.clone());
        if let Some(p)=core.pending.as_mut() {
            if p.mode==Mode::Initial { p.selected=Some(chosen); }
        }
        Ok(Registration::Selected)
    }

    /// Only this native retained AGENT query can mint death. No root check here.
    pub fn observe_agent_death(&self)->Result<AgentDead,DeathError> {
        let (old,base)={
            let core=self.core.try_lock().map_err(|_|DeathError::Busy)?;
            (core.current.clone().ok_or(DeathError::NoCurrent)?,core.epochs)
        };
        let result=old.binding.agent.recheck(); // retained AGENT, never root/metadata
        finish_agent_death_probe(old,base,result)
    }
    /// The fact stays owned by the caller until after the cell guard is dropped.
    pub fn accept_death(&self,fact:AgentDead)->Result<(),DeathError> {
        let result={
            let mut core=self.core.try_lock().map_err(|_|DeathError::Busy)?;
            match &core.current {
                Some(old) if core.epochs==fact.base && Arc::ptr_eq(old,&fact.old)
                    && old.agent_id()==fact.identity=>{
                        old.seal();core.dead.insert(fact.identity);Ok(())
                    }
                _=>Err(DeathError::Superseded),
            }
        };
        drop(fact); // native owners only after the cell mutex is released
        result
    }

    /// Capture the proposal, validate the successful authenticated registration
    /// OUTSIDE the cell, then return a private non-serializable authority object.
    pub fn authorize_change(&self,new_adapter:&str,origin:RegistrationOrigin,
        authorizer:&impl RegistrationAuthorizer)->Result<ChangeGrant,CoreError>
    {
        if !matches!(origin,RegistrationOrigin::ClaimRole|RegistrationOrigin::Reclaim) {
            return Err(CoreError::UnsupportedTransition);
        }
        let (old,base,mut proposed,context)={
            let core=self.try_core()?;
            let old=core.current.clone().ok_or(CoreError::NoCurrent)?;
            let PolicySlot::Fixed(policy)=&core.policy else { return Err(CoreError::InvalidPolicy); };
            if !old.sealed() || !core.dead.contains(&old.agent_id()) { return Err(CoreError::NotDead); }
            let p=core.catalog.iter().find(|p|p.adapter==new_adapter).ok_or(CoreError::InvalidPolicy)?;
            if p.adapter==policy.adapter || p.class!=ProofClass::FullRequired || policy.class!=ProofClass::FullRequired
                || !receiver::BUILTIN_ADAPTERS.contains(&policy.adapter.as_str())
                || !receiver::BUILTIN_ADAPTERS.contains(&p.adapter.as_str()) {
                return Err(CoreError::UnsupportedTransition);
            }
            let context=ChangeContext { surface_id:self.io.surface_id,base:core.epochs,old_adapter:policy.adapter.clone(),
                old_config:policy.config_digest.clone(),new_adapter:p.adapter.clone(),
                new_config:p.config_digest.clone(),origin };
            (old,core.epochs,p.clone(),context)
        };
        let evidence_ref=authorizer.validate_registration(&context).map_err(CoreError::Authority)?;
        if evidence_ref.is_empty() { return Err(CoreError::Authority("missing registration evidence".into())); }
        proposed.policy_epoch=increment(base.policy)?;
        proposed.origin=origin;
        Ok(ChangeGrant { old,base,proposed:Arc::new(proposed),origin,evidence_ref })
    }

    pub fn begin_initial(&self,work:Work)->Result<bool,CoreError> { self.begin(work,Mode::Initial,None) }
    pub fn begin_adoption(&self,work:Work)->Result<bool,CoreError> { self.begin(work,Mode::Adopt,None) }
    pub fn begin_change(&self,work:Work,grant:ChangeGrant)->Result<bool,CoreError> { self.begin(work,Mode::Change,Some(grant)) }
    fn begin(&self,work:Work,mode:Mode,grant:Option<ChangeGrant>)->Result<bool,CoreError> {
        let retired={
            let mut core=self.try_core()?;
            if core.epochs!=work.expected || work.surface_id()!=self.io.surface_id
                || work.obligation_id.is_empty() || work.id().is_empty() { return Err(CoreError::Superseded); }
            if work.cancelled() { return Err(CoreError::Cancelled); }
            let mut prospective=core.epochs;
            prospective.binding=increment(prospective.binding)?;
            let (old,selected,authorization)=match mode {
                Mode::Initial=>{
                    if core.current.is_some() { return Err(CoreError::AlreadyBound); }
                    let selected=match &core.policy { PolicySlot::Catalog=>None,PolicySlot::Fixed(p)=>Some(p.clone()) };
                    (None,selected,None)
                }
                Mode::Adopt|Mode::Change=>{
                    let old=core.current.clone().ok_or(CoreError::NoCurrent)?;
                    if !old.sealed() || !core.dead.contains(&old.agent_id()) { return Err(CoreError::NotDead); }
                    prospective.conversation=increment(prospective.conversation)?;
                    let PolicySlot::Fixed(policy)=&core.policy else { return Err(CoreError::InvalidPolicy); };
                    if mode==Mode::Adopt { (Some(old),Some(policy.clone()),None) }
                    else {
                        let g=grant.as_ref().ok_or(CoreError::UnsupportedTransition)?;
                        if g.base!=core.epochs || !Arc::ptr_eq(&g.old,&old)
                            || !matches!(g.origin,RegistrationOrigin::ClaimRole|RegistrationOrigin::Reclaim)
                            || g.proposed.adapter==policy.adapter
                            || !core.catalog.iter().any(|p|same_contract(p,&g.proposed)) {
                            return Err(CoreError::Superseded);
                        }
                        prospective.policy=increment(prospective.policy)?;
                        if g.proposed.policy_epoch!=prospective.policy { return Err(CoreError::Superseded); }
                        (Some(old),Some(g.proposed.clone()),Some(g.evidence_ref.clone()))
                    }
                }
            };
            if let Some(p)=&core.pending {
                let same_selection=match (&p.selected,&selected) {
                    (Some(a),Some(b))=>same_contract(a,b),(None,None)=>true,_=>false,
                };
                if p.work.same(&work) && p.mode==mode && same_selection {
                    return Ok(false); // no deadline, budget, stage, or policy refresh
                }
                // Expiry retains this logical owner; only explicit same-work recovery may
                // renew the discovery episode. Terminal phases cannot discard a held lease.
                if matches!(p.gate.phase,PublicationPhase::Pending|PublicationPhase::Expired)
                    || p.active_attempt.is_some() || p.staged_attempt.is_some() {
                    return Err(CoreError::PendingConflict);
                }
            }
            let ticket=AttemptTicket { epochs:core.epochs,obligation_id:work.obligation_id.clone(),
                request_id:work.ticket_id(),attempt_id:0 };
            let episode=core.next_episode;core.next_episode=increment(episode)?;
            let p=Pending { episode,work,mode,prospective,selected,old,authorization,
                gate:PublicationGate::new(ticket),active_attempt:None,staged_attempt:None,first_windows:None };
            core.pending.replace(p)
        };
        drop(retired); // any old binding Arcs leave the mutex before destruction
        drop(grant);
        Ok(true)
    }
    pub fn open_window(&self,work:&Work,eligible_at:Instant)->Result<(),CoreError> {
        let mut core=self.try_core()?;let p=core.pending.as_mut().ok_or(CoreError::NoPending)?;
        if !p.work.same(work) { return Err(CoreError::Superseded); }
        if p.gate.budget.eligible_at.is_some_and(|old|old!=eligible_at) { return Err(CoreError::PendingConflict); }
        p.gate.budget.open_window(eligible_at,Duration::from_millis(receiver::BOOT_PROOF_WINDOW_MS));
        Ok(())
    }
    pub fn cancel(&self,work:&Work)->Result<(),CoreError> {
        let mut core=self.try_core()?;let p=core.pending.as_mut().ok_or(CoreError::NoPending)?;
        if !p.work.same(work) { return Err(CoreError::Superseded); }
        p.work.cancel();
        p.gate.cancel(); // does not free executor/native operation or revive another job
        Ok(())
    }
}

impl ReceiverCell {
    /// One native job, one topology collection, one actual-start budget debit.
    /// Completion runs on the bounded worker. It must not spawn an unbounded
    /// waiter or lose the retained request if its own result channel is full.
    pub fn schedule(self:&Arc<Self>,work:Work,
        completion:impl FnOnce(Result<NativeCandidate,CoreError>)+Send+'static)->Result<(),CoreError>
    {
        let episode={
            let core=self.try_core()?;let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
            if !p.work.same(&work) { return Err(CoreError::Superseded); }
            check_pending_phase(p,Instant::now())?;p.episode
        };
        let cell=self.clone();
        self.executor.try_start_guarded(self.io.surface_id,Box::new(move||{
            match cell.begin_actual(&work,episode,Instant::now()) {
                Err(error)=>{ completion(Err(error));None }
                Ok(prepared)=>Some(Box::new(move||completion(cell.run_native(prepared)))),
            }
        }),None).map_err(CoreError::Start)
    }
    /// Only schedule's executor preparation calls this in production. The ticket
    /// cannot be supplied by a caller or minted by numbering two completions.
    fn begin_actual(self:&Arc<Self>,work:&Work,episode:u64,now:Instant)->Result<Prepared,CoreError> {
        let mut core=self.try_core()?;
        if core.epochs!=work.expected { return Err(CoreError::Superseded); }
        let catalog=core.catalog.clone();
        let sealed=core.dead.iter().copied().collect();
        let id=core.next_attempt;let next=increment(id)?;
        let (ticket,selected,prospective,mode)={
            let p=core.pending.as_mut().ok_or(CoreError::NoPending)?;
            if !p.work.same(work) || p.episode!=episode { return Err(CoreError::Superseded); }
            check_pending_phase(p,now)?;
            if p.work.cancelled() { return Err(CoreError::Cancelled); }
            if p.active_attempt.is_some() || p.staged_attempt.is_some() { return Err(CoreError::Start(StartRefusal::SlotBusy)); }
            if let Some(policy)=&p.selected {
                if policy.class==ProofClass::ObservedAgentWeak { return Err(CoreError::WeakServiceRequired((**policy).clone())); }
            }
            p.gate.budget.begin(now).map_err(CoreError::Start)?;
            p.gate.ticket.attempt_id=id;
            p.active_attempt=Some(id);
            (p.gate.ticket.clone(),p.selected.clone(),p.prospective,p.mode)
        };
        core.next_attempt=next;
        Ok(Prepared { lease:ActualAttempt { cell:Arc::downgrade(self),ticket,episode,started_at:now },
            mode,catalog,selected,prospective,sealed })
    }
    fn run_native(&self,prepared:Prepared)->Result<NativeCandidate,CoreError> {
        let provisional=prepared.selected.as_ref().map(|p|p.as_ref())
            .unwrap_or_else(||&prepared.catalog[0]); // only collect; not binding/class selection
        let mut input=self.io.discovery(provisional,prepared.prospective,prepared.sealed.clone());
        let collection=receiver::collect(&input).map_err(CoreError::Native)?;
        self.anchor_collected_root(&prepared.lease.ticket,native_id(collection.root.observation()))?;
        let policy=match &prepared.selected {
            Some(p)=>p.clone(),
            None=>{
                let classified=receiver::classify_collected(&prepared.catalog,&collection).map_err(CoreError::Native)?;
                // The selection is fixed under compare BEFORE any bind_collected.
                self.fix_classification(&prepared.lease.ticket,classified)?
            }
        };
        if policy.class==ProofClass::ObservedAgentWeak { return Err(CoreError::WeakServiceRequired((*policy).clone())); }
        if prepared.mode==Mode::Change {
            // S4-2 requires one STRICT adapter observation, not merely one match
            // for the requested replacement while another adapter is also live.
            // Reuse this SAME collection; no second query or extra start.
            let strict=receiver::classify_collected(&prepared.catalog,&collection).map_err(CoreError::Native)?;
            if !same_contract(&strict,&policy) { return Err(CoreError::Native(Unknown::Ambiguous("different_replacement_declaration".into()))); }
        }
        input.recipe=policy.recipe.clone();input.class=policy.class;
        if policy.policy_epoch!=input.epochs.policy { return Err(CoreError::Superseded); }
        let binding=receiver::bind_collected(&input,collection).map_err(CoreError::Native)?;
        // bind_collected checked root -> chain -> agent -> foreground. Repeat the
        // targeted root last, after every potentially slow observation above.
        binding.root.recheck().map_err(|f|CoreError::Native(Unknown::ValidationFailed(format!("final_root:{f:?}"))))?;
        let summary=Arc::new(NativeSummary { root:binding.root.observation().clone(),
            agent:binding.agent.observation().clone(),ancestors:binding.ancestry.iter().map(|a|a.observation().clone()).collect() });
        Ok(NativeCandidate { lease:prepared.lease,policy,binding:Arc::new(binding),summary,validated_at:Instant::now() })
    }
    fn fix_classification(&self,ticket:&AttemptTicket,selected:ReceiverPolicy)->Result<Arc<ReceiverPolicy>,CoreError> {
        let mut core=self.try_core()?;
        if core.epochs!=ticket.epochs || !core.catalog.iter().any(|p|same_contract(p,&selected)) {
            return Err(CoreError::Superseded);
        }
        let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
        if p.mode!=Mode::Initial || p.gate.ticket!=*ticket || p.active_attempt!=Some(ticket.attempt_id)
            || p.work.cancelled() { return Err(CoreError::Superseded); }
        check_pending_phase(p,Instant::now())?;
        let chosen=match &core.policy {
            PolicySlot::Catalog=>Arc::new(selected),
            PolicySlot::Fixed(p) if same_contract(p,&selected)=>p.clone(),
            _=>return Err(CoreError::Superseded),
        };
        if chosen.policy_epoch!=core.epochs.policy { return Err(CoreError::Superseded); }
        core.policy=PolicySlot::Fixed(chosen.clone());
        core.pending.as_mut().expect("checked above").selected=Some(chosen.clone());
        Ok(chosen)
    }
    fn work_for(&self,ticket:&AttemptTicket)->Result<Work,CoreError> {
        let core=self.try_core()?;let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
        if core.epochs!=ticket.epochs || p.gate.ticket!=*ticket { return Err(CoreError::Superseded); }
        Ok(p.work.clone())
    }
    /// A completed candidate is staged, never installed, before store validation.
    /// No proof constructor can obtain its native token through this API.
    pub fn prepare_publication<V:PublicationValidator>(&self,candidate:NativeCandidate,validator:&V)
        ->Result<PreparedOutcome,CoreError>
    {
        let work=self.work_for(&candidate.lease.ticket)?;
        let transition=self.io.writer.try_transition().ok_or(CoreError::WriterBusy)?;
        let context=CommitContext { work:&work,policy:&candidate.policy,prospective:candidate.binding.epochs,
            root:native_id(&candidate.summary.root),agent:candidate.binding.agent_identity(),native_validated_at:candidate.validated_at };
        let (input_guard,stamp)=validator.acquire_and_validate(&context).map_err(CoreError::Input)?;
        validate_stamp(&work,self.io.surface_id,&stamp)?;
        let proven_zero=work.proven_zero(); // existing Request facts or typed no-write registration
        if !proven_zero { return Err(CoreError::PossibleWrite); }
        let decision={
            let mut core=self.try_core()?;
            validate_candidate(&core,&candidate,self.io.root_pid,Instant::now())?;
            let p=core.pending.as_mut().expect("validated pending");
            if p.gate.budget.deadline.is_none() { return Err(CoreError::WindowNotOpened); }
            if p.staged_attempt.is_some() { return Err(CoreError::PendingConflict); }
            let mut stage_first=false;
            if self.windows && p.mode==Mode::Change {
                let old=p.old.as_ref().expect("change has old");
                match &p.first_windows {
                    None=>stage_first=true,
                    Some(first)=>{
                        if first.ticket.attempt_id==candidate.lease.ticket.attempt_id
                            || candidate.lease.started_at<=first.validated_at
                            || candidate.lease.started_at<=first.started_at {
                            return Err(CoreError::Superseded);
                        }
                        let same=first.base==candidate.lease.ticket.epochs
                            && Arc::ptr_eq(&first.old,old)
                            && first.ticket.request_id==candidate.lease.ticket.request_id
                            && first.ticket.obligation_id==candidate.lease.ticket.obligation_id
                            && same_contract(&first.contract,&candidate.policy)
                            && first.summary.as_ref()==candidate.summary.as_ref();
                        if !same { stage_first=true; }
                    }
                }
                if stage_first {
                    // The old current token still owns these handles. The stage
                    // keeps only Arc identity + plain observations, never a query.
                    p.first_windows=Some(WindowsSample { base:candidate.lease.ticket.epochs,
                        old:old.clone(),contract:candidate.policy.clone(),ticket:candidate.lease.ticket.clone(),
                        started_at:candidate.lease.started_at,validated_at:candidate.validated_at,
                        summary:candidate.summary.clone() });
                }
            }
            if stage_first { None }
            else {
                p.staged_attempt=Some(candidate.lease.ticket.attempt_id);
                Some(TransitionRecord { request_id:work.request_id(),registration_id:work.registration_id(),obligation_id:work.obligation_id.clone(),
                    target:work.target.clone(),old_epochs:work.expected,new_epochs:p.prospective,
                    old_agent:p.old.as_ref().map(|t|t.agent_id()),new_agent:candidate.binding.agent_identity(),root:native_id(&candidate.summary.root),
                    adapter:candidate.policy.adapter.clone(),config_digest:candidate.policy.config_digest.clone(),
                    class:candidate.policy.class,origin:candidate.policy.origin,registration_evidence:p.authorization.clone(),
                    episode:candidate.lease.episode,attempt_id:candidate.lease.ticket.attempt_id,input_generation:stamp.input_generation,
                    resolved_role:stamp.role.clone() })
            }
        };
        drop(input_guard);drop(transition);
        match decision {
            None=>Ok(PreparedOutcome::WindowsFirstSample),
            Some(record)=>{
                let token=Arc::new(PublishedNative::new(candidate.binding.clone()));
                Ok(PreparedOutcome::Prepared(PreparedPublication { candidate,token,record,stamp }))
            }
        }
    }

    /// Call on the bounded observation worker after the matching store operation.
    /// The capability is private, bound to THIS stage/record, and grants no writes
    /// by itself. Native checks occur before reservations/receiver locks; all
    /// fallible compares precede PublicationGate::publish and token activation.
    pub fn activate<V:PublicationValidator>(&self,prepared:PreparedPublication,capability:DurableActivation,
        validator:&V)->Result<ActivatedPublication,CoreError>
    {
        if capability.record!=prepared.record || !Arc::ptr_eq(&capability.token,&prepared.token)
            || capability.reference.is_empty() { return Err(CoreError::Superseded); }
        let c=&prepared.candidate;
        c.binding.check(&*self.io.foreground).map_err(|p|CoreError::Native(Unknown::ValidationFailed(p.reason())))?;
        c.binding.root.recheck().map_err(|f|CoreError::Native(Unknown::ValidationFailed(format!("activation_root:{f:?}"))))?;
        let work=self.work_for(&c.lease.ticket)?;
        let transition=self.io.writer.try_transition().ok_or(CoreError::WriterBusy)?;
        let context=CommitContext { work:&work,policy:&c.policy,prospective:c.binding.epochs,
            root:native_id(&c.summary.root),agent:c.binding.agent_identity(),native_validated_at:Instant::now() };
        let (input_guard,stamp)=validator.acquire_and_validate(&context).map_err(CoreError::Input)?;
        validate_stamp(&work,self.io.surface_id,&stamp)?;
        if !stamp.same_authority(&prepared.stamp) { return Err(CoreError::Input("publication stamps changed during persistence".into())); }
        let proven_zero=work.proven_zero();
        if !proven_zero { return Err(CoreError::PossibleWrite); }
        let displaced={
            let mut core=self.try_core()?;
            validate_candidate(&core,c,self.io.root_pid,Instant::now())?;
            if prepared.token.sealed() || prepared.token.active.load(Ordering::Acquire) { return Err(CoreError::Superseded); }
            let base=core.epochs;
            let p=core.pending.as_mut().expect("validated pending");
            if self.windows && p.mode==Mode::Change && !windows_confirmation_matches(p,c) { return Err(CoreError::Superseded); }
            if p.staged_attempt!=Some(c.lease.ticket.attempt_id) || p.gate.budget.deadline.is_none()
                || p.work.cancelled() { return Err(CoreError::Superseded); }
            let native_valid=c.binding.epochs==p.prospective && c.binding.class==c.policy.class
                && native_id(c.binding.root.observation()).0==self.io.root_pid;
            let input_safe=stamp.same_authority(&prepared.stamp) && stamp.epochs==base;
            // Clock read AFTER lock acquisition and every validation above.
            p.gate.publish(&c.lease.ticket,base,Instant::now(),native_valid,input_safe).map_err(CoreError::Publish)?;
            // No fallible check or native/foreign callback follows this transition.
            p.staged_attempt=None;
            p.active_attempt=None;
            let old=core.current.replace(prepared.token.clone());
            core.epochs=c.binding.epochs;
            core.policy=PolicySlot::Fixed(c.policy.clone());
            prepared.token.active.store(true,Ordering::Release);
            old
        };
        drop(input_guard);drop(transition);drop(displaced);
        Ok(ActivatedPublication { token:prepared.token,record:prepared.record,durable_reference:capability.reference })
    }
}
fn check_pending_phase(p:&Pending,now:Instant)->Result<(),CoreError> {
    match p.gate.phase {
        PublicationPhase::Cancelled=>Err(CoreError::Cancelled),
        PublicationPhase::Expired=>Err(CoreError::Expired),
        PublicationPhase::Published=>Err(CoreError::Superseded),
        PublicationPhase::Pending if p.gate.budget.expired(now)=>Err(CoreError::Expired),
        PublicationPhase::Pending=>Ok(()),
    }
}
fn validate_stamp(work:&Work,surface_id:u64,s:&InputStamp)->Result<(),CoreError> {
    if s.epochs!=work.expected || s.surface_id!=surface_id { return Err(CoreError::Superseded); }
    if let Target::Role(role)=&work.target {
        if s.role.as_ref()!=Some(role) { return Err(CoreError::Input("role no longer resolves to this surface".into())); }
    }
    Ok(())
}
fn validate_candidate(core:&Core,c:&NativeCandidate,root_pid:u32,now:Instant)->Result<(),CoreError> {
    let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
    check_pending_phase(p,now)?;
    if p.work.cancelled() { return Err(CoreError::Cancelled); }
    if core.epochs!=c.lease.ticket.epochs || p.work.expected!=core.epochs || p.episode!=c.lease.episode
        || p.gate.ticket!=c.lease.ticket || p.active_attempt!=Some(c.lease.ticket.attempt_id)
        || c.validated_at<c.lease.started_at || c.binding.epochs!=p.prospective
        || c.policy.policy_epoch!=p.prospective.policy || c.binding.class!=c.policy.class
        || c.policy.class!=ProofClass::FullRequired || native_id(&c.summary.root).0!=root_pid
        || core.root!=Some(native_id(&c.summary.root))
        || native_id(c.binding.root.observation())!=native_id(&c.summary.root) || core.dead.contains(&c.binding.agent_identity())
        || !p.selected.as_ref().is_some_and(|s|same_contract(s,&c.policy)) {
        return Err(CoreError::Superseded);
    }
    match p.mode {
        Mode::Initial=>{
            if core.current.is_some() { return Err(CoreError::AlreadyBound); }
            let PolicySlot::Fixed(policy)=&core.policy else { return Err(CoreError::InvalidPolicy); };
            if !same_contract(policy,&c.policy) { return Err(CoreError::Superseded); }
        }
        Mode::Adopt|Mode::Change=>{
            let old=p.old.as_ref().ok_or(CoreError::NoCurrent)?;
            if !core.current.as_ref().is_some_and(|live|Arc::ptr_eq(live,old))
                || !old.sealed() || !core.dead.contains(&old.agent_id())
                || old.agent_id()==c.binding.agent_identity() { return Err(CoreError::NotDead); }
            let PolicySlot::Fixed(policy)=&core.policy else { return Err(CoreError::InvalidPolicy); };
            if p.mode==Mode::Adopt && !same_contract(policy,&c.policy) { return Err(CoreError::Superseded); }
            if p.mode==Mode::Change && (p.authorization.is_none() || policy.adapter==c.policy.adapter
                || !receiver::BUILTIN_ADAPTERS.contains(&policy.adapter.as_str())
                || !receiver::BUILTIN_ADAPTERS.contains(&c.policy.adapter.as_str())) {
                return Err(CoreError::UnsupportedTransition);
            }
        }
    }
    Ok(())
}

impl ReceiverCell {
    /// Called only from run_native after its accounted collection. No native I/O
    /// occurs in this compare; shell argv/executable is not a lifetime anchor.
    fn anchor_collected_root(&self,ticket:&AttemptTicket,root:NativeId)->Result<(),CoreError> {
        let mut core=self.try_core()?;
        let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
        if core.epochs!=ticket.epochs || p.gate.ticket!=*ticket
            || p.active_attempt!=Some(ticket.attempt_id) || p.work.cancelled()
            || root.0!=self.io.root_pid || (root.1,root.2)==(0,0) { return Err(CoreError::Superseded); }
        check_pending_phase(p,Instant::now())?;
        if core.root.is_some_and(|old|old!=root) { return Err(CoreError::Native(Unknown::RootChanged)); }
        core.root=Some(root);
        Ok(())
    }
    pub fn expire_if_due(&self,work:&Work)->Result<bool,CoreError> {
        let mut core=self.try_core()?;let p=core.pending.as_mut().ok_or(CoreError::NoPending)?;
        if !p.work.same(work) { return Err(CoreError::Superseded); }
        if p.gate.phase==PublicationPhase::Pending && p.gate.budget.expired(Instant::now()) {
            p.gate.phase=PublicationPhase::Expired;return Ok(true);
        }
        Ok(false)
    }
    /// Service calls this only after terminal window disposition and actual query
    /// return/slot release. It drops no work obligation and grants no retry itself.
    /// A later explicit held retry may begin a NEW proof window for the same Work;
    /// same-state begin/open_window calls never silently reset the old window.
    pub fn retire_finished_window(&self,work:&Work)->Result<(),CoreError> {
        let retired={
            let mut core=self.try_core()?;let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
            if !p.work.same(work) { return Err(CoreError::Superseded); }
            if matches!(p.gate.phase,PublicationPhase::Pending|PublicationPhase::Expired) || p.active_attempt.is_some() || p.staged_attempt.is_some() {
                return Err(CoreError::PendingConflict);
            }
            core.pending.take()
        };
        drop(retired);
        Ok(())
    }
}
fn windows_confirmation_matches(p:&Pending,c:&NativeCandidate)->bool {
    let (Some(first),Some(old))=(&p.first_windows,&p.old) else { return false; };
    first.base==c.lease.ticket.epochs && Arc::ptr_eq(&first.old,old)
        && first.ticket.request_id==c.lease.ticket.request_id
        && first.ticket.obligation_id==c.lease.ticket.obligation_id
        && first.ticket.attempt_id!=c.lease.ticket.attempt_id
        && c.lease.started_at>first.validated_at && c.lease.started_at>first.started_at
        && same_contract(&first.contract,&c.policy) && first.summary.as_ref()==c.summary.as_ref()
}

impl ReceiverCell {
    /// Explicit service-owned held retry AFTER backoff/relevant event. The old boot
    /// window stays expired. The same logical owner/IDs/age receive a NEW episode.
    /// Uses the SAME executor Arc installed in this cell; no caller-supplied pool.
    pub fn begin_recovery_episode(&self,work:&Work,eligible_at:Instant)->Result<u64,CoreError> {
        if self.executor.slot_busy(self.io.surface_id) { return Err(CoreError::Start(StartRefusal::SlotBusy)); }
        let (old,episode)={
            let mut core=self.try_core()?;
            let p=core.pending.as_ref().ok_or(CoreError::NoPending)?;
            if !p.work.same(work) || core.epochs!=work.expected { return Err(CoreError::Superseded); }
            if p.work.cancelled() { return Err(CoreError::Cancelled); }
            if p.gate.phase!=PublicationPhase::Expired || p.active_attempt.is_some() || p.staged_attempt.is_some() {
                return Err(CoreError::PendingConflict);
            }
            let episode=core.next_episode;let next=increment(episode)?;
            let ticket=AttemptTicket { epochs:core.epochs,obligation_id:p.work.obligation_id.clone(),
                request_id:p.work.ticket_id(),attempt_id:0 };
            let mut gate=PublicationGate::new(ticket);
            gate.budget.open_window(eligible_at,Duration::from_millis(receiver::BOOT_PROOF_WINDOW_MS));
            let replacement=Pending { episode,work:p.work.clone(),mode:p.mode,prospective:p.prospective,
                selected:p.selected.clone(),old:p.old.clone(),authorization:p.authorization.clone(),
                gate,active_attempt:None,staged_attempt:None,first_windows:None };
            core.next_episode=next;
            (core.pending.replace(replacement),episode)
        };
        drop(old);Ok(episode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn epochs()->Epochs { Epochs { daemon_instance:1,surface_instance:2,launch:3,conversation:4,policy:5,binding:0 } }
    fn policy(adapter:&str)->ReceiverPolicy {
        ReceiverPolicy { policy_epoch:5,declaration:format!("fixture-{adapter}"),config_digest:format!("config-{adapter}"),
            adapter:adapter.into(),recipe:receiver::Recipe { bin:format!("fixture-{adapter}"), declared_invocation:None },
            origin:RegistrationOrigin::HandObserved,class:ProofClass::FullRequired,weak_risk:None }
    }
    fn cell(known:bool)->Arc<ReceiverCell> {
        let p=policy("claude");
        Arc::new(ReceiverCell { io:SeatIo { surface_id:7,root_pid:std::process::id(),
            foreground:Arc::new(||None),writer:WriterGate::default() },executor:Arc::new(Executor::try_new(0).unwrap()),windows:false,
            core:Mutex::new(Core { epochs:epochs(),root:None,catalog:vec![p.clone(),policy("gemini")].into(),
                policy:if known { PolicySlot::Fixed(Arc::new(p)) } else { PolicySlot::Catalog },
                current:None,dead:HashSet::new(),pending:None,next_attempt:1,next_episode:1 }) })
    }
    fn registration_work(c:&ReceiverCell)->Work {
        Work { owner:DiscoveryOwner::Registration(RegistrationObligation::new("reg-fixture".into(),7)),
            obligation_id:"registration-obligation".into(),target:Target::Surface,expected:c.epochs() }
    }

    fn start(c:&Arc<ReceiverCell>,w:&Work,at:Instant)->Result<Prepared,CoreError> {
        let episode=c.core.lock().unwrap().pending.as_ref().unwrap().episode;
        c.begin_actual(w,episode,at)
    }

    #[test]
    fn registration_does_not_require_or_fabricate_a_writer_request() {
        let c=cell(false);let w=registration_work(&c);
        assert!(w.request_id().is_none());assert_eq!(w.registration_id().as_deref(),Some("reg-fixture"));
        assert!(c.begin_initial(w.clone()).unwrap());
        assert_eq!(c.core.lock().unwrap().pending.as_ref().unwrap().gate.ticket.request_id,"registration:reg-fixture");
        assert!(c.current().is_none());
    }
    #[test]
    fn same_contract_registration_preserves_origin_epochs_budget_and_deadline() {
        let c=cell(true);let w=registration_work(&c);let at=Instant::now();
        c.begin_initial(w.clone()).unwrap();c.open_window(&w,at).unwrap();
        let started=start(&c,&w,at).unwrap();drop(started);
        let before={ let s=c.core.lock().unwrap();let p=s.pending.as_ref().unwrap();
            (s.epochs,p.gate.budget.starts,p.gate.budget.deadline,p.gate.ticket.clone()) };
        let mut repeated=policy("claude");repeated.origin=RegistrationOrigin::Reclaim;repeated.policy_epoch=999;
        assert!(matches!(c.register_known(&repeated),Ok(Registration::SameUnbound)));
        let after={ let s=c.core.lock().unwrap();let p=s.pending.as_ref().unwrap();
            (s.epochs,p.gate.budget.starts,p.gate.budget.deadline,p.gate.ticket.clone()) };
        assert_eq!(before,after);
        assert_eq!(c.fixed_policy().unwrap().origin,RegistrationOrigin::HandObserved);
    }
    #[test]
    fn classification_fixes_contract_before_binding_and_survives_a_bind_failure() {
        let c=cell(false);let w=registration_work(&c);let at=Instant::now();
        c.begin_initial(w.clone()).unwrap();c.open_window(&w,at).unwrap();
        let prepared=start(&c,&w,at).unwrap();
        assert!(c.fixed_policy().is_none());
        let selected=c.fix_classification(&prepared.lease.ticket,policy("claude")).unwrap();
        assert_eq!(selected.class,ProofClass::FullRequired);
        assert_eq!(c.fixed_policy().unwrap().adapter,"claude");
        // Simulate the later binder returning Unknown: drop the same attempt without
        // publication. The already selected class/declaration must not be erased.
        drop(prepared);
        assert_eq!(c.fixed_policy().unwrap().class,ProofClass::FullRequired);
        assert!(c.current().is_none());
        assert!(matches!(c.register_known(&policy("gemini")),Ok(Registration::NeedsAuthorizedDifferentBuiltin)));
        assert_eq!(c.fixed_policy().unwrap().adapter,"claude");
    }
    #[test]
    fn work_comparison_covers_all_six_epochs() {
        let c=cell(true);let original=registration_work(&c);
        for axis in 0..6 {
            let mut bad=original.clone();
            match axis { 0=>bad.expected.daemon_instance+=1,1=>bad.expected.surface_instance+=1,
                2=>bad.expected.launch+=1,3=>bad.expected.conversation+=1,
                4=>bad.expected.policy+=1,5=>bad.expected.binding+=1,_=>unreachable!() }
            assert!(matches!(c.begin_initial(bad),Err(CoreError::Superseded)),"axis {axis}");
        }
        assert!(c.begin_initial(original).unwrap());
    }
    #[test]
    fn pre_eligibility_start_and_four_start_budget_are_not_reset_by_registration() {
        let c=cell(true);let w=registration_work(&c);let now=Instant::now();
        c.begin_initial(w.clone()).unwrap();c.open_window(&w,now+Duration::from_secs(1)).unwrap();
        drop(start(&c,&w,now).unwrap());
        assert!(matches!(start(&c,&w,now+Duration::from_millis(300)),Err(CoreError::Start(StartRefusal::BudgetExhausted))));
        c.register_known(&policy("claude")).unwrap();
        for ms in [1000,1250,1500] { drop(start(&c,&w,now+Duration::from_millis(ms)).unwrap()); }
        assert_eq!(c.core.lock().unwrap().pending.as_ref().unwrap().gate.budget.starts,4);
        assert!(matches!(start(&c,&w,now+Duration::from_millis(1750)),Err(CoreError::Start(StartRefusal::BudgetExhausted))));
    }
    #[test]
    fn expired_window_requires_distinct_explicit_recovery_episode() {
        let c=cell(true);let w=registration_work(&c);
        c.begin_initial(w.clone()).unwrap();let old=c.core.lock().unwrap().pending.as_ref().unwrap().episode;
        c.open_window(&w,Instant::now()-Duration::from_secs(3)).unwrap();
        assert!(c.expire_if_due(&w).unwrap());assert!(!c.begin_initial(w.clone()).unwrap());
        assert!(matches!(start(&c,&w,Instant::now()),Err(CoreError::Expired)));
        assert!(matches!(c.retire_finished_window(&w),Err(CoreError::PendingConflict)));
        c.begin_recovery_episode(&w,Instant::now()).unwrap();
        let state=c.core.lock().unwrap();let fresh=state.pending.as_ref().unwrap();
        assert!(fresh.episode>old);assert!(fresh.work.same(&w));assert_eq!(fresh.gate.budget.starts,0);drop(state);
        assert!(matches!(c.begin_actual(&w,old,Instant::now()),Err(CoreError::Superseded)),"old queued preparation entered new episode");
    }

    #[test]
    fn cancellation_keeps_attempt_owned_until_its_lease_returns() {
        let c=cell(true);let w=registration_work(&c);let now=Instant::now();
        c.begin_initial(w.clone()).unwrap();c.open_window(&w,now).unwrap();
        let attempt=start(&c,&w,now).unwrap();c.cancel(&w).unwrap();
        assert!(w.cancelled());
        assert!(matches!(c.retire_finished_window(&w),Err(CoreError::PendingConflict)));
        drop(attempt);c.retire_finished_window(&w).unwrap();
        assert!(matches!(c.begin_initial(w),Err(CoreError::Cancelled)));
    }

    struct FixtureInput { output:std::sync::atomic::AtomicU64 }
    impl FixtureInput { fn new()->Self { Self { output:std::sync::atomic::AtomicU64::new(0) } } }
    impl PublicationValidator for FixtureInput {
        type Guard<'a>=();
        fn acquire_and_validate<'a>(&'a self,c:&CommitContext<'_>)->Result<((),InputStamp),String> {
            // Explicit fixture: NOT a production screen/role/input validator.
            Ok(((),InputStamp { epochs:c.work.expected,surface_id:7,role:None,input_generation:0,
                output_generation:self.output.load(Ordering::Acquire),raw_human_bytes:0,pending_bytes:0,last_human_input:None }))
        }
    }
    struct FixtureStore;
    impl DurableActivationValidator for FixtureStore {
        fn validate_transition_and_retokenization(&self,_:&TransitionRecord)->Result<String,String> {
            Ok("fixture-only:no-real-durable-store".into())
        }
    }
    struct RefusingStore;
    impl DurableActivationValidator for RefusingStore {
        fn validate_transition_and_retokenization(&self,_:&TransitionRecord)->Result<String,String> { Err("no durable record".into()) }
    }

    #[cfg(any(target_os="macos",target_os="linux",windows))]
    #[test]
    fn native_core_stages_privately_and_activates_only_after_matching_capability() {
        let observation=process_observation::observe(std::process::id()).unwrap();
        let bin=std::path::Path::new(std::str::from_utf8(&observation.identity.executable).unwrap())
            .file_name().unwrap().to_str().unwrap().to_string();
        let mut p=policy("claude");p.recipe.bin=bin;p.declaration="explicit-native-test-runner-fixture".into();
        let group=observation.identity.process_group;
        let c=Arc::new(ReceiverCell { io:SeatIo { surface_id:7,root_pid:std::process::id(),
            foreground:Arc::new(move||Some(group)),writer:WriterGate::default() },executor:Arc::new(Executor::try_new(1).unwrap()),windows:false,
            core:Mutex::new(Core { epochs:epochs(),root:None,catalog:vec![p.clone()].into(),
                policy:PolicySlot::Fixed(Arc::new(p)),current:None,dead:HashSet::new(),pending:None,next_attempt:1,next_episode:1 }) });
        let w=registration_work(&c);c.begin_initial(w.clone()).unwrap();
        // Registration prefetch fixture, not a two-second startup timing measurement.
        c.open_window(&w,Instant::now()+Duration::from_secs(20)).unwrap();
        let executor=c.executor.clone();let (tx,rx)=mpsc::channel();
        // Admission is nonblocking (R-C8DORM-1 #6): retry ONLY Busy/AdmissionBusy until a deadline, as the receiver and
        // transition fixtures do; a refused admission starts nothing, so `started` below still counts exactly one start.
        let deadline=Instant::now()+Duration::from_secs(10);
        // R-C8DORM-2 #5: run the retry arm on every run - a holder thread owns the core during the first attempt, so
        // schedule refuses with Busy, the arm sleeps and retries, and an attempt after the release is admitted.
        // (AdmissionBusy takes the same arm; the executor lock that produces it is private to receiver.rs.)
        let (held,holding)=mpsc::channel();let owner=c.clone();
        let holder=std::thread::spawn(move||{
            let core=owner.core.lock().unwrap();held.send(()).unwrap();std::thread::sleep(Duration::from_millis(50));drop(core);
        });
        holding.recv_timeout(Duration::from_secs(10)).expect("core holder started");
        let mut retries=0u32;
        loop {
        let (tx,task_cell)=(tx.clone(),c.clone());
        let scheduled=c.schedule(w.clone(),move|result|{
            // Preserve worker assertion failures instead of letting the executor turn them into
            // a channel timeout, which would not prove the intended mutation discriminator.
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<(),CoreError>{
                let candidate=result?;
                let validator=FixtureInput::new();
                let PreparedOutcome::Prepared(stage)=task_cell.prepare_publication(candidate,&validator)? else { panic!("initial native bind must not require Windows replacement staging"); };
                assert!(task_cell.current().is_none(),"staging installed an executable token");
                assert!(matches!(stage.durable_capability(&RefusingStore),Err(CoreError::Authority(_))));
                assert!(stage.token.check_native(&*task_cell.io.foreground).is_err(),"private staged token admits native proof before durable activation");
                let capability=stage.durable_capability(&FixtureStore)?;
                validator.output.store(2,Ordering::Release); // harmless, freshly validated repaint during persistence
                let active=task_cell.activate(stage,capability,&validator)
                    .expect("freshly safe repaint must remain activatable after durable preparation");
                assert!(active.record.request_id.is_none());assert!(active.record.registration_id.is_some());
                assert!(active.token.check_native(&*task_cell.io.foreground).is_ok());
                assert_eq!(task_cell.epochs().binding,epochs().binding+1);
                active.token.seal();
                assert!(active.token.check_native(&*task_cell.io.foreground).is_err(),"sealed token revived");
                Ok(())
            }));
            tx.send(outcome).unwrap();
        });
        match scheduled {
            Ok(())=>break,
            Err(CoreError::Busy)|Err(CoreError::Start(StartRefusal::AdmissionBusy)) if Instant::now()<deadline=>
                { retries+=1;std::thread::sleep(Duration::from_millis(2)) }
            Err(e)=>panic!("native fixture admission did not become available: {e:?}"),
        }
        }
        holder.join().unwrap();
        assert!(retries>=1,"the Busy/AdmissionBusy retry arm did not run (the core holder did not make the first attempt Busy)");
        drop(tx); // as before the retry: a job dropped without sending disconnects the receiver at once
        match rx.recv_timeout(Duration::from_secs(10)).expect("native fixture completion") {
            Ok(result)=>result.unwrap(),
            Err(panic)=>std::panic::resume_unwind(panic),
        }
        assert_eq!(executor.counters().started.load(Ordering::Acquire),1);
    }

    #[test]
    fn volatile_frame_generation_does_not_replace_authority_checks() {
        let a=InputStamp { epochs:epochs(),surface_id:7,role:Some("worker".into()),input_generation:8,
            output_generation:10,raw_human_bytes:0,pending_bytes:0,last_human_input:None };
        let mut repaint=a.clone();repaint.output_generation=12;
        assert!(a.same_authority(&repaint),"harmless repaint became an output-ack gate");
        for axis in 0..6 {
            let mut b=repaint.clone();match axis {
                0=>b.input_generation+=1,1=>b.raw_human_bytes=1,2=>b.pending_bytes=1,
                3=>b.last_human_input=Some(Instant::now()),4=>b.role=Some("different".into()),
                5=>b.epochs.conversation+=1,_=>unreachable!() }
            assert!(!a.same_authority(&b),"authority axis {axis} ignored");
        }
    }

    #[test]
    fn c8native_expired_work_cannot_be_replaced_around_the_recovery_gate() {
        for outstanding in [false, true] {
            let c=cell(true);let w=registration_work(&c);
            c.begin_initial(w.clone()).unwrap();
            let at=Instant::now()-Duration::from_secs(3);
            c.open_window(&w,at).unwrap();
            let attempt=start(&c,&w,at).unwrap();
            let kept=if outstanding { Some(attempt) } else { drop(attempt);None };
            assert!(c.expire_if_due(&w).unwrap());
            let before={let core=c.core.lock().unwrap();let p=core.pending.as_ref().unwrap();
                (p.episode,p.gate.ticket.clone(),p.gate.budget.starts)};
            let mut replacement=w.clone();replacement.obligation_id="different-obligation".into();
            replacement.target=Target::Role("worker".into());
            let verdict=c.begin_initial(replacement);
            assert!(matches!(verdict,Err(CoreError::PendingConflict)),
                "expired work must remain owned until explicit recovery, outstanding={outstanding}: {verdict:?}");
            let core=c.core.lock().unwrap();let pending=core.pending.as_ref().unwrap();
            assert!(pending.work.same(&w));
            assert_eq!((pending.episode,pending.gate.ticket.clone(),pending.gate.budget.starts),before);
            drop(core);drop(kept);
        }
    }

    #[test]
    fn c8native_cancelled_owner_is_replaceable_only_after_its_attempt_returns() {
        let c=cell(true);let w=registration_work(&c);let at=Instant::now();
        c.begin_initial(w.clone()).unwrap();c.open_window(&w,at).unwrap();
        let attempt=start(&c,&w,at).unwrap();c.cancel(&w).unwrap();
        let fresh=Work { owner:DiscoveryOwner::Registration(
            RegistrationObligation::new("new-registration".into(),7)),
            obligation_id:"new-obligation".into(),target:Target::Surface,expected:c.epochs() };
        assert!(matches!(c.begin_initial(fresh.clone()),Err(CoreError::PendingConflict)),
            "cancellation must not discard an outstanding attempt owner");
        assert!(c.core.lock().unwrap().pending.as_ref().unwrap().work.same(&w));
        drop(attempt);
        assert!(c.begin_initial(fresh.clone()).unwrap(),"returned cancelled attempt must not block new work forever");
        assert!(c.core.lock().unwrap().pending.as_ref().unwrap().work.same(&fresh));
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "native_publication_transition_tests.rs"]
mod native_transition_tests;
