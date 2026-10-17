// Included only inside pending_requests::tests; uses its existing real writer/store fixtures.
#[test]
fn c8aftercare_zero_recovery_persists_before_release_and_never_replays() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-order");let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw("original").with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();
    assert!(journal.reconcile_completion().is_err(),"no real writer completion exists yet");
    disk.fail.store(true,Ordering::Release);let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    assert!(journal.reconcile_completion().is_err());assert!(!journal.has_stored_outcome());
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),"failed persistence released native ownership");
    disk.fail.store(false,Ordering::Release);
    assert_eq!(journal.reconcile_completion().expect("aftercare must persist before resolving"),attempts::CompletionRecovery::ZeroWriteResolved,
        "producer recovery must store the original receipt before resolving its reservation");
    assert!(bytes.is_empty());assert!(journal.has_stored_outcome());
    assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::ZeroWriteResolved);
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(PassThrough)).unwrap();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::WrittenSubmitted{..}));assert_eq!(bytes,b"original\r");
}

#[test]
fn c8aftercare_automatic_zero_release_acknowledges_only_unchanged_ownership() {
    use crate::agent_write::{self,Terminal};
    for (cancel,next) in [(false,"attempt"),(false,"edit"),(true,"none")] {
        let scratch=Scratch::new("aftercare-auto");let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare")).unwrap();
        let current=attempt_current(&store,&cap,Plan::raw("owned"));let p=attempt_prepare(&store,&current,None);let journal=p.journal();
        let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();
        if cancel {current.request().cancel();}
        let terminal=agent_write::decline_unstarted(job,"queue_full");
        assert!(matches!(terminal,Terminal::RefusedNoWrite{..}|Terminal::CancelledBeforeWrite));
        assert_eq!(journal.reconcile_completion().expect("automatically released zero needs idempotent acknowledgement"),attempts::CompletionRecovery::ZeroWriteResolved,
            "an already automatically released zero result must not report WrongLease");
        assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::ZeroWriteResolved);
        assert_eq!(current.request().is_cancelled(),cancel);
        if !cancel {
            if next=="attempt" {drop(current.request().reserve_attempt(current.plan(),None).unwrap());}
            else {drop(current.request().reserve_zero_write_edit().unwrap());}
            // Exercise the actual receipt resolver before the higher-level supersession result.
            assert!(journal.resolve_stored_zero_write().is_err(),
                "old actual acknowledgement survived new {next} ownership");
            assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::SupersededAttempt { number:1 },
                "old completion needs a terminal journal disposition after new {next} ownership");
        }
    }
}

fn aftercare_summary_facts(rows:&[attempts::AttemptSummary])->Vec<serde_json::Value> {
    rows.iter().map(|s|serde_json::json!({"number":s.number,"phase":s.phase,"outcome":s.outcome,
        "accepted":s.accepted,"native":s.native,"consumed":s.consumed})).collect()
}

#[test]
fn c8aftercare_resumed_prefix_roundtrip_and_corruption_negatives_never_release() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-resume");let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw(&"x".repeat(10000)).with_submit(0,0));
    let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(AttemptCut(std::sync::atomic::AtomicUsize::new(0)))).unwrap();
    let mut bytes=Vec::new();let first=attempt_execute(&mut bytes,&job);let prefix=first.envelope;
    assert!(matches!(agent_write::finish(job,&first),Terminal::PartialExact{..}));let old_bytes=bytes.clone();
    let p=attempt_prepare(&store,&current,Some(prefix));let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();
    disk.fail.store(true,Ordering::Release);let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::PartialUnknown{..}));
    assert!(matches!(current.request().reserve_attempt(current.plan(),Some(prefix)),Err(LeaseError::Busy)),"premise: failed finish retains original native ownership");
    disk.fail.store(false,Ordering::Release);
    assert_eq!(journal.reconcile_completion().expect("partial outcome aftercare must store without release"),attempts::CompletionRecovery::StoredWithoutNativeRelease {kind:attempts::OutcomeKind::PartialExact});
    assert!(matches!(current.request().reserve_attempt(current.plan(),Some(prefix)),Err(LeaseError::Busy)),"storage aftercare released a nonzero native hold");
    assert_eq!(bytes,old_bytes);let prior=store.attempt_summaries(current.id()).unwrap();
    assert_eq!(prior.last().unwrap().outcome,Some(attempts::OutcomeKind::PartialExact));
    assert!(prior.iter().all(|r|!r.recovered));
    let image:serde_json::Value=serde_json::from_slice(&fs::read(scratch.0.join(DATA)).unwrap()).unwrap();
    drop(store);
    assert!(matches!(journal.reconcile_completion(),Err(ProtocolError::Inactive)),
        "old nonzero journal stayed live after its store dropped");
    let reopened=PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap();
    assert!(matches!(journal.reconcile_completion(),Err(ProtocolError::Inactive)),
        "reopen revived the old nonzero journal");
    let reloaded=reopened.attempt_summaries(current.id()).unwrap();
    assert_eq!(aftercare_summary_facts(&reloaded),aftercare_summary_facts(&prior),"persisted original prefix did not round-trip");
    assert!(reloaded.iter().all(|r|r.recovered));
    assert!(matches!(current.request().reserve_attempt(current.plan(),Some(prefix)),Err(LeaseError::Busy)));
    assert!(reopened.current_revision(&current,attempt_revision(&reopened,&current)).is_err(),"reopened data reconstructed live authority");
    let unchanged=Scratch::new("aftercare-unchanged-copy");
    write_private(&unchanged.0.join(DATA),&serde_json::to_vec(&image).unwrap());
    let control=PendingStore::open_with_registry(&unchanged.0,11,attempt_store_limits(),registry_limits())
        .expect("unmodified image must pass the same private-file opener");
    assert_eq!(aftercare_summary_facts(&control.attempt_summaries(current.id()).unwrap()),aftercare_summary_facts(&prior));
    for case in ["calls","changed-envelope"] {
        let bad=Scratch::new("aftercare-corrupt");let mut changed=image.clone();
        let outcome=&mut changed["rows"][0]["attempts"]["entries"]["2"]["outcome"];
        if case=="calls" {outcome["accepted"]["calls"]=1.into();outcome["native"]["run_accepted"]["calls"]=1.into();}
        else {let offset=outcome["envelope"]["body_offset"].as_u64().unwrap()+1;outcome["envelope"]["body_offset"]=offset.into();outcome["native"]["envelope"]["body_offset"]=offset.into();}
        write_private(&bad.0.join(DATA),&serde_json::to_vec(&changed).unwrap());
        let result=PendingStore::open_with_registry(&bad.0,11,attempt_store_limits(),registry_limits());
        let expected=if case=="calls" {"attempt bytes without Writing"} else {"attempt progress bounds"};
        assert!(matches!(&result,Err(StoreError::Invalid(reason)) if *reason==expected),
            "corrupt resumed {case} did not reach its intended validator: {:?}",result.err());
    }
}
