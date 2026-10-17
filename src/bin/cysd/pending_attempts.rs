//! Durable managed attempt records. Recorded values are data, never replay permits.
//! The existing writer owns native progress; this module validates its private receipts.
use super::*;
use crate::agent_write::{Accepted,AgentJob,AttemptLease,AttemptStamp,AttemptStartReceipt,Envelope,Journal,NativeProgress,Plan,Proof,ProofContext,Request,Terminal,WriterReceipt};
use std::time::{SystemTime,UNIX_EPOCH};

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttemptLimits { pub max_records:usize,pub max_record_bytes:usize,pub diagnostic_preview_bytes:usize }
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum AttemptPhase { Prepared,Writing,Outcome }
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum OutcomeKind { RefusedNoWrite,CancelledBeforeWrite,WrittenBody,WrittenSubmitted,PartialExact,PartialUnknown }
#[derive(Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Diagnostic { bytes:usize,sha256:String,preview:String }
impl fmt::Debug for Diagnostic {
    fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result {
        f.debug_struct("Diagnostic").field("bytes",&self.bytes).field("sha256",&self.sha256)
            .field("preview_bytes",&self.preview.len()).finish()
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct StartFacts { envelope:Envelope,native:NativeProgress,resumed:bool,at_ms:u64 }
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Outcome {
    kind:OutcomeKind,envelope:Envelope,accepted:Accepted,native:NativeProgress,reason:Option<Diagnostic>,
    writing_confirmed:bool,writing_uncertain:bool,at_ms:u64,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptRecord {
    number:u64,prepared_revision:u64,plan_digest:String,selected:SelectedLineage,
    before:NativeProgress,resume:Option<Envelope>,resume_parent:Option<u64>,reserved_bytes:usize,
    phase:AttemptPhase,writing:Option<StartFacts>,outcome:Option<Outcome>,consumed:bool,prepared_at_ms:u64,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AttemptLedger {
    limits:AttemptLimits,next_number:u64,first_partial:Option<u64>,entries:BTreeMap<u64,AttemptRecord>,
}
#[derive(Clone,Debug)]
pub(crate) struct AttemptSummary {
    pub number:u64,pub phase:AttemptPhase,pub outcome:Option<OutcomeKind>,pub accepted:Option<Accepted>,
    pub native:Option<NativeProgress>,pub consumed:bool,pub recovered:bool,
}
/// Result of actual completion aftercare, not a native/write/source-settlement permit.
#[derive(Debug,PartialEq,Eq)]
pub(crate) enum CompletionRecovery {
    ZeroWriteResolved,
    StoredWithoutNativeRelease { kind:OutcomeKind },
    /// Stop retrying THIS journal's completion recovery. No request/source retirement,
    /// new-owner release, durable-success claim or execution permission is implied.
    /// It is not "recovered": never match it together with the two variants above.
    SupersededAttempt { number:u64 },
}

fn epoch_ms()->u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0,|v|u64::try_from(v.as_millis()).unwrap_or(u64::MAX)) }
fn diagnostic(reason:&str,limit:usize)->Diagnostic {
    let mut n=reason.len().min(limit);while !reason.is_char_boundary(n) { n-=1; }
    Diagnostic { bytes:reason.len(),sha256:raw_digest(reason),preview:reason[..n].to_owned() }
}
fn accepted_total(a:Accepted)->Result<u64,StoreError> {
    a.payload.checked_add(a.fence).and_then(|n|n.checked_add(a.control)).ok_or(StoreError::Invalid("attempt count overflow"))
}
fn validate_limits(l:AttemptLimits,store:Limits)->Result<(),StoreError> {
    if l.max_records<3 || l.max_records>64 || l.max_record_bytes<512 || l.max_record_bytes>store.max_record_bytes
        || l.diagnostic_preview_bytes>512 { return Err(StoreError::Invalid("attempt limits")); }
    Ok(())
}
fn outcome_of(r:&WriterReceipt<'_>,limits:AttemptLimits)->Outcome {
    let (kind,accepted,reason)=match r.terminal() {
        Terminal::RefusedNoWrite { reason }=>(OutcomeKind::RefusedNoWrite,r.native_progress().run_accepted,Some(reason.as_str())),
        Terminal::CancelledBeforeWrite=>(OutcomeKind::CancelledBeforeWrite,r.native_progress().run_accepted,None),
        Terminal::WrittenBody { accepted }=>(OutcomeKind::WrittenBody,*accepted,None),
        Terminal::WrittenSubmitted { accepted }=>(OutcomeKind::WrittenSubmitted,*accepted,None),
        Terminal::PartialExact { accepted,reason,.. }=>(OutcomeKind::PartialExact,*accepted,Some(reason.as_str())),
        Terminal::PartialUnknown { accepted,reason,.. }=>(OutcomeKind::PartialUnknown,*accepted,Some(reason.as_str())),
    };
    Outcome { kind,envelope:r.envelope(),accepted,native:r.native_progress(),reason:reason.map(|s|diagnostic(s,limits.diagnostic_preview_bytes)),
        writing_confirmed:r.writing_confirmed(),writing_uncertain:r.writing_uncertain(),at_ms:epoch_ms() }
}
/// Validate the durable projection against the exact Plan, including partial-phase lower
/// bounds. A PartialUnknown can include a partly accepted fence/control, so its envelope
/// is only a lower bound; resumable and successful results must be byte-exact.
fn validate_progress(plan:&ExactPlan,n:NativeProgress,exact:bool)->Result<(),StoreError> {
    let e=n.envelope;let body=plan.body.as_ref().map_or(0,String::len);
    let control=plan.control.as_ref().map_or(0,Vec::len);
    let fenced=plan.fenced&&plan.body.is_some();
    if e.body_offset>body || (e.cleared&&!plan.clear_first) || ((e.opened||e.closed)&&!fenced)
        || (e.control_done&&plan.control.is_none()) || (e.submitted&&plan.submit.is_none())
        || (plan.clear_first&&!e.cleared&&(e.opened||e.body_offset>0||e.closed||e.control_done||e.submitted))
        || (fenced&&!e.opened&&(e.body_offset>0||e.closed||e.control_done||e.submitted))
        || (e.closed&&e.body_offset!=body)
        || ((e.control_done||e.submitted)&&(e.body_offset!=body||(fenced&&!e.closed)))
        || (e.submitted&&control>0&&!e.control_done) {
        return Err(StoreError::Invalid("attempt envelope/plan"));
    }
    let open=cys::paste_fence::OPEN.len() as u64;let close=cys::paste_fence::CLOSE.len() as u64;
    let bound=Accepted { payload:body as u64,fence:if fenced {open+close}else{0},
        control:(control as u64).checked_add(u64::from(plan.clear_first)+u64::from(plan.submit.is_some())).ok_or(StoreError::Invalid("attempt plan size"))?,calls:0 };
    let prefix=Accepted { payload:e.body_offset as u64,fence:u64::from(e.opened)*open+u64::from(e.closed)*close,
        control:u64::from(e.cleared)+if e.control_done {control as u64}else{0}+u64::from(e.submitted),calls:0 };
    let known=accepted_total(prefix)?;let run=accepted_total(n.run_accepted)?;
    if n.accepted_total>accepted_total(bound)? || known>n.accepted_total || (exact&&known!=n.accepted_total)
        || run>n.accepted_total || n.run_accepted.payload>bound.payload || n.run_accepted.fence>bound.fence
        || n.run_accepted.control>bound.control || (run>0&&n.run_accepted.calls==0) {
        return Err(StoreError::Invalid("attempt progress bounds"));
    }
    Ok(())
}
fn validate_outcome(plan:&ExactPlan,a:&AttemptRecord,o:&Outcome,limits:AttemptLimits)->Result<(),StoreError> {
    validate_progress(plan,o.native,o.kind!=OutcomeKind::PartialUnknown)?;
    if matches!(o.kind,OutcomeKind::WrittenBody|OutcomeKind::WrittenSubmitted) {
        let complete=Envelope { cleared:plan.clear_first,opened:plan.fenced&&plan.body.is_some(),
            body_offset:plan.body.as_ref().map_or(0,String::len),closed:plan.fenced&&plan.body.is_some(),
            control_done:plan.control.as_ref().is_some_and(|v|!v.is_empty()),submitted:plan.submit.is_some() };
        if o.envelope!=complete || (o.kind==OutcomeKind::WrittenSubmitted)!=plan.submit.is_some() {
            return Err(StoreError::Invalid("attempt incomplete success"));
        }
    }
    if o.native.outstanding || o.envelope!=o.native.envelope || o.accepted!=o.native.run_accepted
        || o.native.accepted_total<a.before.accepted_total {
        return Err(StoreError::Invalid("attempt native outcome"));
    }
    if accepted_total(o.accepted)?!=o.native.accepted_total-a.before.accepted_total {
        return Err(StoreError::Invalid("attempt accepted delta"));
    }
    if let Some(d)=&o.reason {
        if d.preview.len()>limits.diagnostic_preview_bytes || d.preview.len()>d.bytes || d.sha256.len()!=64
            || !d.sha256.bytes().all(|c|c.is_ascii_hexdigit()) { return Err(StoreError::Invalid("attempt diagnostic")); }
    }
    if matches!(o.kind,OutcomeKind::RefusedNoWrite|OutcomeKind::CancelledBeforeWrite) && !o.native.proven_zero() {
        return Err(StoreError::Invalid("attempt false zero"));
    }
    if o.kind==OutcomeKind::PartialExact && (o.native.unknown || o.native.proven_zero()) {
        return Err(StoreError::Invalid("attempt false exact cut"));
    }
    if matches!(o.kind,OutcomeKind::WrittenBody|OutcomeKind::WrittenSubmitted) && o.native.unknown {
        return Err(StoreError::Invalid("attempt unknown success"));
    }
    if o.kind==OutcomeKind::WrittenSubmitted && !o.envelope.submitted { return Err(StoreError::Invalid("attempt absent submit")); }
    if o.writing_confirmed && a.writing.is_none() { return Err(StoreError::Invalid("attempt Writing receipt absent")); }
    // A failed Writing barrier or undispatched decline can retain the accepted parent prefix.
    // It owns no new native call/byte; storing that fact never releases the held lease.
    let unchanged_resume=a.resume==Some(o.envelope) && a.resume_parent.is_some()
        && o.native.accepted_total==a.before.accepted_total && o.accepted==Accepted::default()
        && !o.writing_confirmed;
    if a.writing.is_none() && (accepted_total(o.accepted)?>0 || matches!(o.kind,OutcomeKind::WrittenBody|OutcomeKind::WrittenSubmitted)
        || (o.kind==OutcomeKind::PartialExact&&!unchanged_resume)) {
        return Err(StoreError::Invalid("attempt bytes without Writing"));
    }
    Ok(())
}
/// Serialization-only upper bound, never a real outcome or execution capability.
/// Every variable diagnostic byte uses the JSON worst-case control-character encoding.
fn fund_outcome(a:&AttemptRecord,limits:AttemptLimits)->Result<(),StoreError> {
    let accepted=Accepted { payload:u64::MAX,fence:u64::MAX,control:u64::MAX,calls:u64::MAX };
    let envelope=Envelope { body_offset:usize::MAX,..Envelope::default() };
    let native=NativeProgress { accepted_total:u64::MAX,envelope,run_accepted:accepted,..NativeProgress::default() };
    let mut largest=a.clone();largest.phase=AttemptPhase::Outcome;largest.consumed=false;
    largest.writing=Some(StartFacts { envelope,native,resumed:false,at_ms:u64::MAX });
    largest.outcome=Some(Outcome { kind:OutcomeKind::CancelledBeforeWrite,envelope,accepted,native,
        reason:Some(Diagnostic { bytes:usize::MAX,sha256:"f".repeat(64),preview:"\0".repeat(limits.diagnostic_preview_bytes) }),
        writing_confirmed:false,writing_uncertain:false,at_ms:u64::MAX });
    encode_limited(&largest,a.reserved_bytes).map_err(|_|StoreError::Capacity("unfunded attempt completion"))?;
    Ok(())
}
pub(super) fn validate_ledger(row:&Row,store:Limits)->Result<(),StoreError> {
    let Some(ledger)=&row.attempts else { return Ok(()); };
    let RecordLineage::Protocol { data }=&row.record.lineage else { return Err(StoreError::Invalid("attempts on legacy row")); };
    validate_limits(ledger.limits,store)?;
    if data.selected.is_none() && !ledger.entries.is_empty() { return Err(StoreError::Invalid("attempts without selected lineage")); }
    let active:Vec<_>=ledger.entries.iter().filter(|(_,a)|a.phase!=AttemptPhase::Outcome).map(|(n,_)|*n).collect();
    if active.len()>1 || active.first().is_some_and(|n|Some(n)!=ledger.entries.keys().next_back()) {
        return Err(StoreError::Invalid("multiple or superseded active attempts"));
    }
    if ledger.next_number==0 || ledger.entries.len()>ledger.limits.max_records { return Err(StoreError::Invalid("attempt count/highwater")); }
    let latest=ledger.entries.keys().next_back().copied().ok_or(StoreError::Invalid("empty attempt ledger"))?;
    if latest.checked_add(1)!=Some(ledger.next_number) { return Err(StoreError::Invalid("attempt highwater gap")); }
    let digest=raw_digest(&String::from_utf8(encode_limited(&row.record.plan,store.max_record_bytes)?).map_err(|_|StoreError::Invalid("plan UTF8"))?);
    for (number,a) in &ledger.entries {
        if *number==0 || *number!=a.number || *number>=ledger.next_number || a.prepared_revision==0 || a.prepared_revision>data.revision
            || a.reserved_bytes!=ledger.limits.max_record_bytes || a.plan_digest!=digest || a.before.outstanding || a.before.unknown {
            return Err(StoreError::Invalid("attempt identity/plan"));
        }
        selected_shape(&row.record,data,&a.selected)?;
        validate_progress(&row.record.plan,a.before,true)?;
        if a.consumed!=(*number!=latest) { return Err(StoreError::Invalid("attempt consumption order")); }
        if a.resume.is_some()!=a.resume_parent.is_some() || a.resume_parent.is_some_and(|parent|parent>=*number) {
            return Err(StoreError::Invalid("attempt resume parent order"));
        }
        if a.resume!=Some(a.before.envelope) && !(a.resume.is_none()&&a.before.proven_zero()) { return Err(StoreError::Invalid("attempt resume start")); }
        if let Some(w)=&a.writing {
            if w.envelope!=a.resume.unwrap_or_default() || w.native.envelope!=w.envelope || w.native.outstanding || w.native.unknown
                || w.native.accepted_total!=a.before.accepted_total || w.native.run_accepted!=Accepted::default() || w.resumed!=a.resume.is_some() {
                return Err(StoreError::Invalid("attempt Writing start"));
            }
        }
        match (a.phase,&a.writing,&a.outcome) {
            (AttemptPhase::Prepared,None,None)|(AttemptPhase::Writing,Some(_),None)=>{},
            (AttemptPhase::Outcome,_,Some(outcome))=>validate_outcome(&row.record.plan,a,outcome,ledger.limits)?,
            _=>return Err(StoreError::Invalid("attempt phase")),
        }
        if a.consumed && a.phase!=AttemptPhase::Outcome { return Err(StoreError::Invalid("consumed active attempt")); }
        if a.phase!=AttemptPhase::Outcome {
            if let Some(parent)=a.resume_parent {
                let parent=ledger.entries.get(&parent).ok_or(StoreError::Invalid("attempt missing live resume parent"))?;
                if parent.selected!=a.selected || !parent.consumed || !parent.outcome.as_ref().is_some_and(|o|
                    o.kind==OutcomeKind::PartialExact&&o.native==a.before&&Some(o.envelope)==a.resume) {
                    return Err(StoreError::Invalid("attempt parent progress"));
                }
            }
        }
        encode_limited(a,a.reserved_bytes)?;
        if a.phase!=AttemptPhase::Outcome { fund_outcome(a,ledger.limits)?; }
    }
    let first_retained_partial=ledger.entries.iter().find(|(_,a)|a.outcome.as_ref().is_some_and(|o|o.kind==OutcomeKind::PartialExact)).map(|(n,_)|*n);
    if ledger.first_partial!=first_retained_partial {
        return Err(StoreError::Invalid("attempt lost first partial anchor"));
    }
    Ok(())
}
pub(super) fn reserved_extra(row:&Row,store:Limits)->Result<usize,StoreError> {
    let Some(ledger)=&row.attempts else { return Ok(0); };
    let mut extra=20usize.checked_sub(ledger.next_number.to_string().len()).ok_or(StoreError::Invalid("attempt counter width"))?;
    // first_partial is absent until the first exact cut. Reserve its maximum-width value.
    if ledger.first_partial.is_none() { extra=extra.checked_add(16).ok_or(StoreError::Capacity("attempt reserve"))?; }
    for a in ledger.entries.values().filter(|a|a.phase!=AttemptPhase::Outcome) {
        let used=encode_limited(a,store.max_record_bytes)?.len();
        extra=extra.checked_add(a.reserved_bytes.checked_sub(used).ok_or(StoreError::Capacity("attempt outcome reserve"))?)
            .ok_or(StoreError::Capacity("attempt reserve"))?;
    }
    Ok(extra)
}
fn prune_one_consumed(ledger:&mut AttemptLedger,keep:u64)->bool {
    let protected_parent=ledger.entries.get(&keep).and_then(|a|a.resume_parent);
    let latest_outcome=ledger.entries.iter().rev().find(|(_,a)|a.phase==AttemptPhase::Outcome).map(|(n,_)|*n);
    let victim=ledger.entries.iter().find(|(n,a)|**n!=keep && Some(**n)!=ledger.first_partial && Some(**n)!=protected_parent
        && Some(**n)!=latest_outcome && a.consumed).map(|(n,_)|*n);
    if let Some(victim)=victim { ledger.entries.remove(&victim);true } else { false }
}
fn prune_history(ledger:&mut AttemptLedger,keep:u64)->Result<(),StoreError> {
    while ledger.entries.len()>ledger.limits.max_records {
        if !prune_one_consumed(ledger,keep) { return Err(StoreError::Capacity("protected attempt history")); }
    }
    Ok(())
}

/// One post-persistence preparation. The lease is held by VALUE through preparation IO.
/// Dropping a never-started preparation releases native exclusion but leaves its durable
/// Prepared row protected for explicit no-dispatch reconciliation; it never authorizes replay.
pub(crate) struct PreparedAttempt { lease:AttemptLease,journal:Arc<AttemptJournal>,resume:Option<Envelope> }
impl PreparedAttempt {
    pub fn journal(&self)->Arc<AttemptJournal> { self.journal.clone() }
    pub fn number(&self)->u64 { self.journal.number }
    pub fn into_job(self,proof:Arc<dyn Proof>)->Result<AgentJob,ProtocolError> {
        self.journal.live_store()?;
        self.journal.current.initial_registration.check_live()?;
        Ok(AgentJob { request:self.journal.current.request.clone(),attempt:Some(self.lease),plan:self.journal.current.plan.as_ref().clone(),
            proof,journal:Some(self.journal),resume:self.resume,settle:None,guard:None,note_body:false,body_owner:None })
    }
}
/// Issued only from this run's retained real writer receipt AND its committed outcome.
/// No public constructor/Clone/Deserialize; releasing this exclusion is not write authority.
pub(crate) struct DurableZeroWriteOutcome {
    owner:Weak<RuntimeIdentity>,current:Arc<CurrentRequest>,stamp:AttemptStamp,native:NativeProgress,
}
impl DurableZeroWriteOutcome {
    pub(crate) fn consume(self,request:&Arc<Request>)->Result<(AttemptStamp,NativeProgress),ProtocolError> {
        check_resolution_owner(&self.owner,&self.current,request,&self.current.plan)?;
        Ok((self.stamp,self.native))
    }
}
struct RuntimeOutcome { actual:Option<Outcome>,stored:Option<Durability> }
/// Only actual started/completed writer receipts reach this journal. Legacy methods reject.
/// Retain this Arc in the owning controller until the outcome and required source effects close.
pub(crate) struct AttemptJournal {
    store:Weak<PendingStore>,current:Arc<CurrentRequest>,stamp:AttemptStamp,number:u64,
    limits:AttemptLimits,outcome:Mutex<RuntimeOutcome>,
}
impl AttemptJournal {
    fn live_store(&self)->Result<Arc<PendingStore>,ProtocolError> {
        let store=self.store.upgrade().ok_or(ProtocolError::Inactive)?;
        let owner=self.current.owner.upgrade().ok_or(ProtocolError::Inactive)?;
        if !Arc::ptr_eq(&owner,&store.identity) || !owner.healthy.load(Ordering::Acquire) { return Err(ProtocolError::Inactive); }
        Ok(store)
    }
    fn row<'a>(&self,store:&PendingStore,state:&'a State)->Result<&'a Row,ProtocolError> {
        let row=state.rows.get(self.current.id()).ok_or(ProtocolError::Missing)?;
        let RecordLineage::Protocol { data }=&row.record.lineage else { return Err(ProtocolError::Conflict); };
        store.validate_current_locked(state,&self.current,data.revision)?;
        let entry=row.attempts.as_ref().and_then(|l|l.entries.get(&self.number)).ok_or(ProtocolError::Missing)?;
        if entry.number!=self.number || !plan_matches(&row.record.plan,&self.current.plan) || data.selected.as_ref()!=Some(&entry.selected) {
            return Err(ProtocolError::Conflict);
        }
        Ok(row)
    }
    fn persist_outcome(&self,actual:&Outcome)->Result<Durability,ProtocolError> {
        let store=self.live_store()?;let mut state=store.try_mutation()?;let row=self.row(&store,&state)?;
        let ledger=row.attempts.as_ref().ok_or(ProtocolError::Missing)?;let entry=ledger.entries.get(&self.number).ok_or(ProtocolError::Missing)?;
        if let Some(old)=&entry.outcome {
            if old!=actual { return Err(ProtocolError::Conflict); }
            return self.outcome.lock().unwrap_or_else(|e|e.into_inner()).stored.ok_or(ProtocolError::Conflict);
        }
        validate_outcome(&row.record.plan,entry,actual,ledger.limits)?;
        let mut rows=(*state.rows).clone();let row=rows.get_mut(self.current.id()).unwrap();
        let RecordLineage::Protocol { data }=&mut row.record.lineage else { unreachable!() };
        data.revision=data.revision.checked_add(1).ok_or(StoreError::Capacity("request revision exhausted"))?;
        let ledger=row.attempts.as_mut().unwrap();let entry=ledger.entries.get_mut(&self.number).unwrap();
        entry.phase=AttemptPhase::Outcome;entry.outcome=Some(actual.clone());
        if actual.kind==OutcomeKind::PartialExact && ledger.first_partial.is_none() { ledger.first_partial=Some(self.number); }
        let mut registry=state.registry.clone();let nr=state.next_request_seq;let ns=state.next_settle_seq;
        let bytes=prune_and_encode(&mut rows,nr,ns,store.limits,None,&mut registry,store.registry_limits)?;
        let durability=store.commit(&mut state,rows,nr,ns,registry,&bytes)?;Ok(durability)
    }
    /// Retry storing retained ACTUAL completion facts; never execute a body/Return again.
    /// Native exclusion remains held after a failed writer finish until a separate typed
    /// resolution consumes the committed outcome; this method alone grants no retry permission.
    pub(super) fn retry_outcome(&self)->Result<(),ProtocolError> {
        self.retry_outcome_core(|| {})
    }
    // Commit, then publish. Production supplies a no-op; the cfg(test) accessor observes
    // the committed-but-unpublished interval of this same algorithm.
    fn retry_outcome_core(&self, between:impl FnOnce())->Result<(),ProtocolError> {
        let actual={self.outcome.lock().unwrap_or_else(|e|e.into_inner()).actual.clone().ok_or(ProtocolError::Missing)?};
        let durability=self.persist_outcome(&actual)?;
        between();
        self.outcome.lock().unwrap_or_else(|e|e.into_inner()).stored=Some(durability);Ok(())
    }
    pub(super) fn zero_write_receipt(&self)->Result<DurableZeroWriteOutcome,ProtocolError> {
        let actual={
            let retained=self.outcome.lock().unwrap_or_else(|e|e.into_inner());
            retained.stored.ok_or(ProtocolError::Missing)?;
            retained.actual.clone().ok_or(ProtocolError::Missing)?
        };
        if !matches!(actual.kind,OutcomeKind::RefusedNoWrite|OutcomeKind::CancelledBeforeWrite)
            || !actual.native.proven_zero() { return Err(ProtocolError::Unsupported); }
        let store=self.live_store()?;let state=store.try_mutation()?;let row=self.row(&store,&state)?;
        let ledger=row.attempts.as_ref().ok_or(ProtocolError::Missing)?;
        let entry=ledger.entries.get(&self.number).ok_or(ProtocolError::Missing)?;
        if ledger.entries.keys().next_back()!=Some(&self.number) || entry.phase!=AttemptPhase::Outcome
            || entry.outcome.as_ref()!=Some(&actual) { return Err(ProtocolError::Conflict); }
        Ok(DurableZeroWriteOutcome { owner:Arc::downgrade(&store.identity),current:self.current.clone(),
            stamp:self.stamp.clone(),native:actual.native })
    }
    /// Explicit current-run resolution after storage retry. The writer checks the exact
    /// original Started operation; no status, cancellation, Plan or source state is reset.
    pub(super) fn resolve_stored_zero_write(&self)->Result<(),ProtocolError> {
        self.current.request.resolve_durable_zero_write(self.zero_write_receipt()?)?;Ok(())
    }
    fn superseded(&self)->Result<bool,ProtocolError> {
        // A reopened store never substitutes for this journal's original live owner.
        let _store=self.live_store()?;
        Ok(self.stamp.ownership(&self.current.request)?==crate::agent_write::AttemptOwnership::Superseded)
    }
    /// Producer-facing recovery. Component steps are visible to pending_requests and
    /// its descendants, not to sibling handlers. A positive newer ownership nonce is
    /// a terminal disposition for THIS journal only; no generic Conflict is permanent.
    /// Otherwise retained actual facts are persisted before typed native-zero release.
    /// Partial/unknown/success storage does not resolve native or source effects here.
    pub fn reconcile_completion(&self)->Result<CompletionRecovery,ProtocolError> {
        self.reconcile_completion_core(|| {},|| {})
    }
    // One algorithm. Production supplies only no-ops; the cfg(test) accessors can place
    // a real ownership or store transition at these exact boundaries without a hook registry.
    fn reconcile_completion_core(&self, after_entry:impl FnOnce(), after_sequence:impl FnOnce())
        ->Result<CompletionRecovery,ProtocolError> {
        let actual=self.outcome.lock().unwrap_or_else(|e|e.into_inner()).actual.clone()
            .ok_or(ProtocolError::Missing)?;
        if self.superseded()? { return Ok(CompletionRecovery::SupersededAttempt { number:self.number }); }
        after_entry(); // No store/native/outcome mutex is held here.
        let result:Result<CompletionRecovery,ProtocolError>=(|| {
            self.retry_outcome()?;
            if matches!(actual.kind,OutcomeKind::RefusedNoWrite|OutcomeKind::CancelledBeforeWrite)
                && actual.native.proven_zero() {
                self.resolve_stored_zero_write()?;
                Ok(CompletionRecovery::ZeroWriteResolved)
            } else {
                Ok(CompletionRecovery::StoredWithoutNativeRelease { kind:actual.kind })
            }
        })();
        after_sequence(); // No store/native/outcome mutex is held here either.
        // New ownership may have begun after the entry check. The nonce relation is
        // monotonic for this stamp; checking it does not touch either owner's state.
        // A store lost here turns a completed release into Err (pinned by a specimen).
        match self.superseded() {
            Ok(true)=>Ok(CompletionRecovery::SupersededAttempt { number:self.number }),
            Ok(false)=>result,
            Err(error)=>match result { Err(original)=>Err(original),Ok(_)=>Err(error) },
        }
    }
    pub fn has_stored_outcome(&self)->bool { self.stored_durability().is_some() }
    pub fn stored_durability(&self)->Option<Durability> { self.outcome.lock().unwrap_or_else(|e|e.into_inner()).stored }
}
fn io_protocol(error:ProtocolError)->io::Error { io::Error::other(format!("managed attempt: {error:?}")) }
impl Journal for AttemptJournal {
    fn writing(&self,_:&Request)->io::Result<()> { Err(io::Error::other("private managed start receipt required")) }
    fn terminal(&self,_:&Request,_:&Terminal,_:&Envelope)->io::Result<()> { Err(io::Error::other("private managed completion receipt required")) }
    fn writing_attempt(&self,ctx:&ProofContext<'_>,receipt:Option<&AttemptStartReceipt<'_>>)->io::Result<()> {
        let start=receipt.ok_or_else(||io::Error::other("missing managed start"))?.for_attempt(&self.stamp)
            .map_err(|e|io::Error::other(format!("managed start: {e:?}")))?;
        if !std::ptr::eq(start.request(),ctx.request) || start.plan()!=ctx.plan || start.resume().unwrap_or_default()!=ctx.envelope
            || ctx.accepted!=Accepted::default() || ctx.phase_offset!=0 || ctx.resumed!=start.resume().is_some() {
            return Err(io::Error::other("actual writer context differs"));
        }
        let native=ctx.request.native_progress();
        let facts=StartFacts { envelope:ctx.envelope,native,resumed:ctx.resumed,at_ms:epoch_ms() };
        let result=(||->Result<(),ProtocolError>{
            let store=self.live_store()?;let mut state=store.try_mutation()?;let row=self.row(&store,&state)?;
            let entry=&row.attempts.as_ref().unwrap().entries[&self.number];
            if entry.phase!=AttemptPhase::Prepared { return Err(ProtocolError::Conflict); }
            if native.outstanding || native.unknown || native.envelope!=ctx.envelope || native.run_accepted!=Accepted::default()
                || native.accepted_total!=entry.before.accepted_total || entry.resume!=start.resume() { return Err(ProtocolError::Conflict); }
            let mut rows=(*state.rows).clone();let row=rows.get_mut(self.current.id()).unwrap();
            let RecordLineage::Protocol { data }=&mut row.record.lineage else { unreachable!() };
            data.revision=data.revision.checked_add(1).ok_or(StoreError::Capacity("request revision exhausted"))?;
            let entry=row.attempts.as_mut().unwrap().entries.get_mut(&self.number).unwrap();entry.phase=AttemptPhase::Writing;entry.writing=Some(facts);
            let mut registry=state.registry.clone();let nr=state.next_request_seq;let ns=state.next_settle_seq;
            let bytes=prune_and_encode(&mut rows,nr,ns,store.limits,None,&mut registry,store.registry_limits)?;
            store.commit(&mut state,rows,nr,ns,registry,&bytes)?;Ok(())
        })();result.map_err(io_protocol)
    }
    fn terminal_receipt(&self,receipt:&WriterReceipt<'_>)->io::Result<()> {
        receipt.for_attempt(&self.stamp).map_err(|e|io::Error::other(format!("managed completion: {e:?}")))?;
        let mut actual=outcome_of(receipt,self.limits);
        {
            let mut pending=self.outcome.lock().unwrap_or_else(|e|e.into_inner());
            if let Some(old)=&pending.actual {
                actual.at_ms=old.at_ms; // repeat storage never refreshes the actual completion time
                if old!=&actual { return Err(io::Error::other("completion already captured")); }
            } else { pending.actual=Some(actual.clone()); }
        }
        let durability=self.persist_outcome(&actual).map_err(io_protocol)?;
        self.outcome.lock().unwrap_or_else(|e|e.into_inner()).stored=Some(durability);Ok(())
    }
}

impl PendingStore {
    pub fn prepare_attempt(self:&Arc<Self>,current:&Arc<CurrentRequest>,expected:u64,
        lease:AttemptLease,resume:Option<Envelope>,limits:AttemptLimits)->Result<PreparedAttempt,ProtocolError> {
        validate_limits(limits,self.limits)?;
        let stamp=lease.validate_prepared(&current.request,&current.plan,resume)?;
        let before=current.request.native_progress();
        let mut state=self.try_mutation()?;let (row,data)=self.validate_current_locked(&state,current,expected)?;
        self.validate_live_registration(&state,&current.initial_registration)?;
        if row.terminal.is_some() || data.ingress.target_kind!=TargetKind::Agent
            || matches!(data.ingress.submit,SubmitContract::AwaitingAssociation { .. }|SubmitContract::BoundToCompletedBody { .. }) {
            return Err(ProtocolError::Unsupported);
        }
        let selected=data.selected.clone().ok_or(ProtocolError::SelectionRequired)?;
        let mut ledger=row.attempts.clone().unwrap_or(AttemptLedger { limits,next_number:1,first_partial:None,entries:BTreeMap::new() });
        if ledger.limits!=limits { return Err(ProtocolError::Conflict); }
        let mut parent=None;
        if let Some((number,previous))=ledger.entries.iter_mut().next_back() {
            let outcome=previous.outcome.as_ref().ok_or(ProtocolError::AttemptOutstanding)?;
            match (outcome.kind,resume) {
                (OutcomeKind::RefusedNoWrite,None) if before.proven_zero()=>{},
                (OutcomeKind::PartialExact,Some(from)) if from==outcome.envelope && previous.selected==selected
                    && before.accepted_total==outcome.native.accepted_total && !before.unknown && !before.outstanding=>{parent=Some(*number);},
                _=>return Err(ProtocolError::Unsupported),
            }
            previous.consumed=true;
        } else if resume.is_some() || !before.proven_zero() { return Err(ProtocolError::Conflict); }
        let number=ledger.next_number;ledger.next_number=number.checked_add(1).ok_or(StoreError::Capacity("attempt sequence exhausted"))?;
        // Prepared, Writing and Outcome must all fit before any native bytes may run.
        data.revision.checked_add(3).ok_or(StoreError::Capacity("request revision exhausted"))?;
        let revision=data.revision+1;
        let digest=raw_digest(&String::from_utf8(encode_limited(&row.record.plan,self.limits.max_record_bytes)?)
            .map_err(|_|StoreError::Invalid("plan UTF8"))?);
        ledger.entries.insert(number,AttemptRecord { number,prepared_revision:revision,plan_digest:digest,selected,before,resume,resume_parent:parent,
            reserved_bytes:limits.max_record_bytes,phase:AttemptPhase::Prepared,writing:None,outcome:None,consumed:false,prepared_at_ms:epoch_ms() });
        prune_history(&mut ledger,number)?;
        let mut rows=(*state.rows).clone();let row=rows.get_mut(current.id()).unwrap();row.attempts=Some(ledger);
        let RecordLineage::Protocol { data }=&mut row.record.lineage else { unreachable!() };data.revision=revision;
        let mut registry=state.registry.clone();let nr=state.next_request_seq;let ns=state.next_settle_seq;
        // Count and byte caps are independent. Reclaim eligible attempt history on the
        // candidate only; current, first-partial and live-resume-parent records stay pinned.
        let bytes=loop {
            match prune_and_encode(&mut rows,nr,ns,self.limits,None,&mut registry,self.registry_limits) {
                Ok(bytes)=>break bytes,
                Err(error@StoreError::Capacity(_))=>{
                    let ledger=rows.get_mut(current.id()).and_then(|row|row.attempts.as_mut()).ok_or(ProtocolError::Missing)?;
                    if !prune_one_consumed(ledger,number) { return Err(error.into()); }
                },
                Err(error)=>return Err(error.into()),
            }
        };
        self.commit(&mut state,rows,nr,ns,registry,&bytes)?;
        let journal=Arc::new(AttemptJournal { store:Arc::downgrade(self),current:current.clone(),stamp,number,limits,
            outcome:Mutex::new(RuntimeOutcome { actual:None,stored:None }) });
        Ok(PreparedAttempt { lease,journal,resume })
    }
    pub fn attempt_summaries(&self,id:&str)->Result<Vec<AttemptSummary>,StoreError> {
        let snapshot=self.snapshot();if snapshot.uncertain { return Err(StoreError::UncertainCommit); }
        let row=snapshot.rows.get(id).ok_or(StoreError::Missing)?;
        Ok(row.attempts.as_ref().map(|ledger|ledger.entries.values().map(|a|AttemptSummary { number:a.number,phase:a.phase,
            outcome:a.outcome.as_ref().map(|o|o.kind),accepted:a.outcome.as_ref().map(|o|o.accepted),native:a.outcome.as_ref().map(|o|o.native),
            consumed:a.consumed,recovered:snapshot.loaded.contains(id) }).collect()).unwrap_or_default())
    }
}

#[cfg(test)]
impl AttemptJournal {
    pub(super) fn test_reconcile_after_entry(&self, after_entry:impl FnOnce())->Result<CompletionRecovery,ProtocolError> {
        self.reconcile_completion_core(after_entry,|| {})
    }
    pub(super) fn test_reconcile_after_sequence(&self, after_sequence:impl FnOnce())->Result<CompletionRecovery,ProtocolError> {
        self.reconcile_completion_core(|| {},after_sequence)
    }
    /// The production retry_outcome algorithm with its commit-to-publication interval
    /// observed. Only a test callback is inserted; no receipt or durability is fabricated.
    pub(super) fn test_commit_then_publish(&self, between:impl FnOnce())->Result<(),ProtocolError> {
        self.retry_outcome_core(between)
    }
}
#[cfg(test)]
#[path="pending_aftercare_envelope_tests.rs"]
mod aftercare_envelope_tests;
