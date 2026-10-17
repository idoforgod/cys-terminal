// Included inside pending_requests::tests. Real store/writer IO with existing synthetic
// upstream admission/proof fixtures; no native/live-surface or source-settlement claim.
fn batch2_assert_superseded(journal:&attempts::AttemptJournal, current:&Arc<CurrentRequest>,
    store:&PendingStore, root:&Path, number:u64) {
    let bytes=fs::read(root.join(DATA)).unwrap();
    let native=current.request().native_progress();let plan=current.plan().clone();
    let at=current.received_at();let cancelled=current.request().is_cancelled();
    let facts=aftercare_summary_facts(&store.attempt_summaries(current.id()).unwrap());
    assert_eq!(journal.reconcile_completion().expect("old journal needs a terminal disposition"),
        attempts::CompletionRecovery::SupersededAttempt { number },
        "old completion did not become typed SupersededAttempt");
    assert_eq!(fs::read(root.join(DATA)).unwrap(),bytes,"superseded aftercare mutated the current store row");
    assert_eq!(aftercare_summary_facts(&store.attempt_summaries(current.id()).unwrap()),facts);
    assert_eq!(current.request().native_progress(),native,"superseded aftercare changed current native facts");
    assert_eq!(current.plan(),&plan);assert_eq!(current.received_at(),at);
    assert_eq!(current.request().is_cancelled(),cancelled);
}

#[test]
fn c8aftercare_old_journal_is_typed_after_held_dropped_or_finished_new_owners() {
    use crate::agent_write::{self,Terminal,LeaseError};
    for case in ["held-attempt","dropped-attempt","held-edit","dropped-edit","finished-zero","started-zero"] {
        let scratch=Scratch::new(case);let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
        let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-batch2")).unwrap();
        let current=attempt_current(&store,&cap,Plan::raw("same admitted plan"));
        let p=attempt_prepare(&store,&current,None);let number=p.number();let journal=p.journal();
        let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();
        assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::RefusedNoWrite{..}));
        assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::ZeroWriteResolved);
        let mut held_attempt=None;let mut held_edit=None;let mut second=None;
        match case {
            "held-attempt"=>held_attempt=Some(current.request().reserve_attempt(current.plan(),None).unwrap()),
            "dropped-attempt"=>drop(current.request().reserve_attempt(current.plan(),None).unwrap()),
            "held-edit"=>held_edit=Some(current.request().reserve_zero_write_edit().unwrap()),
            "dropped-edit"=>drop(current.request().reserve_zero_write_edit().unwrap()),
            "finished-zero"=>{let p=attempt_prepare(&store,&current,None);let job=p.into_job(Arc::new(NoDispatchProof)).unwrap();
                assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::RefusedNoWrite{..}));},
            "started-zero"=>second=Some(reconcile_failed_zero_finish(&store,&current,&disk)),
            _=>unreachable!(),
        }
        batch2_assert_superseded(&journal,&current,&store,&scratch.0,number);
        if held_attempt.is_some() || held_edit.is_some() || second.is_some() {
            assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),
                "old journal released the new owner");
        } else { drop(current.request().reserve_attempt(current.plan(),None).unwrap()); }
        // Cancellation is not an exemption that lets stale aftercare change a newer lease.
        current.request().cancel();
        batch2_assert_superseded(&journal,&current,&store,&scratch.0,number);
        assert!(current.request().is_cancelled(),"superseded aftercare revived cancellation");
        drop(held_attempt);drop(held_edit);drop(second);
    }
}

#[test]
fn c8aftercare_committed_row_before_runtime_publication_is_not_superseded() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-publication-gap");let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-batch2")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw("original"));let p=attempt_prepare(&store,&current,None);
    let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();
    disk.fail.store(true,Ordering::Release);let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    disk.fail.store(false,Ordering::Release);
    journal.test_commit_then_publish(||{
        assert_eq!(store.attempt_summaries(current.id()).unwrap().last().unwrap().phase,attempts::AttemptPhase::Outcome);
        assert!(!journal.has_stored_outcome(),"premise: runtime publication must still be pending");
        // Here the committed-entry group is satisfied, so the stored check is the only guard.
        assert!(matches!(journal.zero_write_receipt(),Err(ProtocolError::Missing)),
            "zero-release receipt issued before this run published its own stored durability");
        assert!(matches!(journal.resolve_stored_zero_write(),Err(ProtocolError::Missing)));
        assert!(matches!(journal.reconcile_completion(),Err(ProtocolError::Conflict)),
            "a committed-but-unpublished outcome was permanently retired");
        assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)));
    }).unwrap();
    assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::ZeroWriteResolved);
    assert!(bytes.is_empty());
}

#[test]
fn c8aftercare_old_zero_journal_stays_inactive_after_drop_and_reopen() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-dropped-zero");let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-batch2")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw("held"));let p=attempt_prepare(&store,&current,None);
    let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();
    disk.fail.store(true,Ordering::Release);let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    disk.fail.store(false,Ordering::Release);journal.retry_outcome().unwrap();
    assert!(journal.has_stored_outcome(),"premise: a genuine outcome was persisted before dropping the store");
    let native=current.request().native_progress();let image=fs::read(scratch.0.join(DATA)).unwrap();drop(store);
    assert!(matches!(journal.zero_write_receipt(),Err(ProtocolError::Inactive)),
        "stored actual receipt lost its original-owner liveness guard");
    assert!(matches!(journal.reconcile_completion(),Err(ProtocolError::Inactive)),
        "old zero journal stayed live after its store dropped");
    let reopened=PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap();
    // Structure regression pin: the journal holds Weak<PendingStore> to the dropped allocation,
    // so no reopen can be adopted by construction; these two asserts pin that type property.
    assert!(matches!(journal.zero_write_receipt(),Err(ProtocolError::Inactive)),
        "reopen substituted a new runtime for the old receipt owner");
    assert!(matches!(journal.reconcile_completion(),Err(ProtocolError::Inactive)),
        "same-path reopen revived the old zero journal");
    assert!(reopened.attempt_summaries(current.id()).unwrap().iter().all(|row|row.recovered));
    assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),image);assert_eq!(current.request().native_progress(),native);
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),
        "reopened data released old native exclusion");
    assert!(bytes.is_empty());
}

#[test]
fn c8aftercare_store_lost_after_release_reports_inactive_over_completed_release() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-store-lost-after-release");let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-batch2")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw("held"));let p=attempt_prepare(&store,&current,None);
    let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();
    disk.fail.store(true,Ordering::Release);let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    disk.fail.store(false,Ordering::Release);
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),
        "premise: failed persistence keeps the native exclusion held for this reconcile to release");
    let mut owned=Some(store);let mut dropped=false;
    let result=journal.test_reconcile_after_sequence(|| { drop(owned.take()); dropped=true; });
    assert!(dropped,"premise: the store must be lost after the release sequence ran");
    // Current decision, pinned: this reconcile DID release, yet the caller sees Inactive.
    assert!(matches!(result,Err(ProtocolError::Inactive)),
        "store lost after a completed release no longer reports Inactive: {result:?}");
    drop(current.request().reserve_attempt(current.plan(),None)
        .expect("premise: this reconcile really released the native exclusion"));
    assert!(bytes.is_empty());
}

#[test]
fn c8aftercare_actual_receipt_guards_reject_release_before_persistence() {
    use crate::agent_write::{self,PassThrough,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-actual-guards");let disk=Arc::new(SwitchDisk {fail:AtomicBool::new(false)});
    let store=Arc::new(PendingStore::open_registry_with(&scratch.0,11,attempt_store_limits(),Some(registry_limits()),disk.clone()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-batch2")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw("held"));let p=attempt_prepare(&store,&current,None);
    let journal=p.journal();let job=p.into_job(Arc::new(PassThrough)).unwrap();
    disk.fail.store(true,Ordering::Release);let mut bytes=Vec::new();let ex=attempt_execute(&mut bytes,&job);
    assert!(matches!(agent_write::finish(job,&ex),Terminal::RefusedNoWrite{..}));
    assert!(!journal.has_stored_outcome());
    assert!(journal.resolve_stored_zero_write().is_err(),
        "actual failed persistence minted and consumed a zero-release receipt");
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),
        "receipt guard removal released before the store committed");
    disk.fail.store(false,Ordering::Release);
    assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::ZeroWriteResolved);
    assert!(bytes.is_empty());
}

#[test]
fn c8aftercare_actual_latest_guard_refuses_old_receipt_issuance() {
    use crate::agent_write::{self,Terminal,LeaseError};
    let scratch=Scratch::new("aftercare-latest-guard");
    let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
    let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-batch2")).unwrap();
    let current=attempt_current(&store,&cap,Plan::raw("held"));let first=attempt_prepare(&store,&current,None);
    let journal=first.journal();let job=first.into_job(Arc::new(NoDispatchProof)).unwrap();
    assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::RefusedNoWrite{..}));
    let second=attempt_prepare(&store,&current,None);assert_eq!(second.number(),2);
    assert!(journal.zero_write_receipt().is_err(),"old journal issued a latest-attempt receipt");
    assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)));
    drop(second);
}


#[test]
fn c8aftercare_post_entry_supersession_preserves_real_new_owner() {
    use crate::agent_write::{self,Terminal,LeaseError};
    for case in ["held-attempt","held-edit","dropped-attempt","dropped-edit"] {
        let scratch=Scratch::new(case);
        let store=Arc::new(PendingStore::open_with_registry(&scratch.0,11,attempt_store_limits(),registry_limits()).unwrap());
        let cap=store.register_catalog(registry_scope(),fresh_registry_catalog("aftercare-post-entry")).unwrap();
        let current=attempt_current(&store,&cap,Plan::raw("same admitted plan"));
        let first=attempt_prepare(&store,&current,None);let number=first.number();let journal=first.journal();
        let job=first.into_job(Arc::new(NoDispatchProof)).unwrap();
        assert!(matches!(agent_write::decline_unstarted(job,"queue_full"),Terminal::RefusedNoWrite{..}));
        assert_eq!(journal.reconcile_completion().unwrap(),attempts::CompletionRecovery::ZeroWriteResolved,
            "premise: no newer owner exists before the entry check");
        let stored=fs::read(scratch.0.join(DATA)).unwrap();let native=current.request().native_progress();
        let plan=current.plan().clone();let at=current.received_at();let cancelled=current.request().is_cancelled();
        let mut entered=false;let mut held_attempt=None;let mut held_edit=None;
        let result=journal.test_reconcile_after_entry(|| {
            entered=true;
            // This is called by the SAME core only after its non-superseded entry
            // check, and before its original retry_outcome/zero-release sequence.
            match case {
                "held-attempt"=>held_attempt=Some(current.request().reserve_attempt(current.plan(),None).unwrap()),
                "held-edit"=>held_edit=Some(current.request().reserve_zero_write_edit().unwrap()),
                "dropped-attempt"=>drop(current.request().reserve_attempt(current.plan(),None).unwrap()),
                "dropped-edit"=>drop(current.request().reserve_zero_write_edit().unwrap()),
                _=>unreachable!(),
            }
        });
        assert!(entered,"the entry check skipped the forced ownership interval");
        assert_eq!(fs::read(scratch.0.join(DATA)).unwrap(),stored,"old recovery changed durable state after newer ownership");
        assert_eq!(current.request().native_progress(),native);assert_eq!(current.plan(),&plan);
        assert_eq!(current.received_at(),at);assert_eq!(current.request().is_cancelled(),cancelled);
        if let Some(lease)=held_attempt.as_ref() {
            assert!(lease.validate_prepared(current.request(),current.plan(),None).is_ok(),
                "old recovery invalidated the real new attempt lease");
        }
        if let Some(lease)=held_edit.as_ref() {
            assert!(lease.validate_reserved(current.request(),current.plan()).is_ok(),
                "old recovery invalidated the real new edit lease");
        }
        if held_attempt.is_some() || held_edit.is_some() {
            assert!(matches!(current.request().reserve_attempt(current.plan(),None),Err(LeaseError::Busy)),
                "old recovery released the real new owner");
        }
        assert!(matches!(result,Ok(attempts::CompletionRecovery::SupersededAttempt { number:n }) if n==number),
            "post-entry newer ownership was not returned as typed SupersededAttempt: {result:?}");
        // After the assertion, prove that an intentionally dropped owner did not
        // create a hold. Held leases above stay owned until their explicit Drop.
        if held_attempt.is_none() && held_edit.is_none() {
            drop(current.request().reserve_attempt(current.plan(),None).unwrap());
        }
        drop(held_attempt);drop(held_edit);
    }
}
