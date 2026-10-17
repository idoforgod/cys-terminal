// Data-only corruption fixture for the persisted validator, NOT a writer receipt,
// native-success factory, or claim that this partial path is reachable by today's writer.
use super::*;
#[test]
fn equal_total_empty_control_envelope_flip_reaches_resume_equality_guard() {
    let plan=ExactPlan {clear_first:true,clear_settle_ms:0,body:None,fenced:false,
        control:Some(vec![]),submit:None};
    let prefix=Envelope {cleared:true,..Envelope::default()};
    let before=NativeProgress {accepted_total:1,envelope:prefix,..NativeProgress::default()};
    let record=AttemptRecord {number:2,prepared_revision:3,plan_digest:"data-only-unused".into(),
        selected:SelectedLineage {catalog_id:"data-only".into(),adapter:"fixture".into(),config_digest:"fixture".into(),
            epochs:Epochs {daemon_instance:1,surface_instance:1,launch:1,conversation:1,policy:1,binding:1},
            class:ProofClass::FullRequired,weak_risk:None,publication_ref:"data-only".into()},
        before,resume:Some(prefix),resume_parent:Some(1),reserved_bytes:8192,phase:AttemptPhase::Outcome,
        writing:None,outcome:None,consumed:false,prepared_at_ms:1};
    let limits=AttemptLimits {max_records:3,max_record_bytes:8192,diagnostic_preview_bytes:64};
    let original=Outcome {kind:OutcomeKind::PartialExact,envelope:prefix,accepted:Accepted::default(),
        native:before,reason:None,writing_confirmed:false,writing_uncertain:true,at_ms:2};
    validate_progress(&plan,original.native,true).expect("original envelope must meet plan/count bounds");
    validate_outcome(&plan,&record,&original,limits).expect("unchanged zero-new-byte prefix must be structurally admissible");
    let mut changed=original.clone();changed.envelope.control_done=true;changed.native.envelope=changed.envelope;
    assert_eq!(changed.native.accepted_total,original.native.accepted_total);
    validate_progress(&plan,changed.native,true).expect("empty control flip must still meet earlier plan/count bounds");
    assert!(matches!(validate_outcome(&plan,&record,&changed,limits),Err(StoreError::Invalid("attempt bytes without Writing"))),
        "equal-total empty-control envelope change bypassed resumed-prefix equality");
}
