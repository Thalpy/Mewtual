use super::*;
use crate::chat::owner_bodies::{
    ExactReviewDecisionBody, ExactReviewOriginal, ReviewDecision, ReviewDecisionBody, ReviewSpec,
    ReviewStreamEntry, ReviewStreamPosition, ReviewStreamRecord, UnsignedReviewStreamEntry,
};
use crate::chat::owner_header::OwnerStateCommitment;
use zeroize::Zeroizing;

fn genesis_pin(fixture: &Fixture) -> FounderGenesisAnchorData {
    let replay = fixture.verifier.replay_data();
    assert_eq!(replay.mls_epoch_data(), 0);
    FounderGenesisAnchorData {
        group: replay.profile().spec().group.clone(),
        founder: replay.profile().spec().founder,
        profile: replay.profile().id(),
        admission: fixture.verifier.admissions[0].core.body.admission_id(),
        tenure: replay.profile().tenure(),
        header: replay.header().id(),
        state: replay.header().spec().state,
    }
}

fn freeze(fixture: &mut Fixture, admission: &V2AuthenticatedAdmission) -> V2FrozenAdmission {
    let mut removed = fixture.remove(admission);
    assert!(removed.take_close_completion().unwrap().try_complete());
    removed.take_frozen_data().unwrap()
}

fn decision(
    evidence: &V2RemovedCreateEvidence,
    intent: u8,
    choice: ReviewDecision,
    previous: Option<OwnerBodyId>,
) -> ExactReviewDecisionBody {
    let (remove_header, remove_epoch) = evidence.removal_data();
    ExactReviewDecisionBody::new(
        ReviewDecisionBody::new(ReviewSpec {
            target: evidence.target_data().clone(),
            intent: [intent; 16],
            decision: choice,
            previous,
        })
        .unwrap(),
        ExactReviewOriginal {
            digest: evidence.original_digest_data(),
            remove_header,
            remove_epoch,
        },
    )
    .unwrap()
}

fn public_record(
    owner: &Owner,
    pin: &FounderGenesisAnchorData,
    position: ReviewStreamPosition,
    decision: &ExactReviewDecisionBody,
    held: &[u8],
) -> (ReviewStreamEntry, Zeroizing<Vec<u8>>) {
    let prepared = UnsignedReviewStreamEntry::prepare_public(pin, position, decision).unwrap();
    let signature = owner.key.sign(prepared.signing_bytes());
    let entry = prepared.attach_signature(pin, signature).unwrap();
    let frame = ReviewStreamRecord::encode(pin, &entry, decision, held).unwrap();
    (entry, frame)
}

#[test]
fn reviewed_create_waits_for_verified_parent_and_has_no_projection_effect() {
    let mut f = Fixture::new();
    let pin = genesis_pin(&f);
    let key = DeviceKeypair::from_seed(&[107; 32]);
    let other = DeviceKeypair::from_seed(&[108; 32]);
    let author = f.add(&key, 30);
    let parent_author = f.add(&other, 31);
    let parent = create(&other, &parent_author, 1);
    let reply = action(
        &key,
        &author,
        2,
        Body::Create {
            text: "Reviewed reply".into(),
            reply_to: Some(parent.id()),
        },
        vec![parent.id()],
    );
    let original = reply.encode();
    let frozen = freeze(&mut f, &author);
    let membership = (f.verifier.replay_data().header().id(), f.epoch());
    let (mut ledger, view) = f.ledger();
    let evidence = f
        .verifier
        .verify_removed_create_for_review(&original, &frozen, &ledger, &view, &[])
        .unwrap();
    let held = evidence.encode_record_data(&original).unwrap();
    let position = ReviewStreamPosition {
        sequence: 1,
        previous: None,
        membership_header: membership.0,
        membership_epoch: membership.1,
    };
    let approval = decision(&evidence, 45, ReviewDecision::Approve, None);
    let (selected, frame) = public_record(&f.owner, &pin, position, &approval, &held);
    let record = ReviewStreamRecord::decode_public_approval(&pin, selected.id(), &frame).unwrap();
    ledger.import(parent.clone()).unwrap();
    assert!(matches!(
        f.verifier.check_selected_reviewed_create(
            (&pin, membership), (&selected, &record), &frozen, (&ledger, &view), &[],
        ),
        Ok(V2ReviewedCreateCheck::WaitingForReplyParent(id)) if id == parent.id()
    ));
    assert_eq!(ledger.state(reply.id()), None);

    assert_eq!(
        f.verifier
            .project_provisional_action(&mut ledger, &view, &parent, &parent_author, &[], 32),
        Ok(ActionState::Valid)
    );
    let V2ReviewedCreateCheck::Ready(checked) = f
        .verifier
        .check_selected_reviewed_create(
            (&pin, membership),
            (&selected, &record),
            &frozen,
            (&ledger, &view),
            &[],
        )
        .unwrap()
    else {
        panic!("the real selected parent should make the proof ready");
    };
    assert!(std::ptr::eq(checked.record_data(), &record));
    assert_eq!(ledger.state(reply.id()), None);
    f.verifier
        .require_current_reviewed_create(&checked, &selected)
        .unwrap();

    let mut replacement = Fixture::new();
    let replacement_author = replacement.add(&key, 30);
    replacement.add(&other, 31);
    freeze(&mut replacement, &replacement_author);
    assert_eq!(
        replacement.verifier.replay_data().header().id(),
        membership.0
    );
    assert_eq!(
        replacement
            .verifier
            .require_current_reviewed_create(&checked, &selected),
        Err(ChatError::Scope)
    );

    f.add(&DeviceKeypair::from_seed(&[109; 32]), 32);
    assert_eq!(
        f.verifier
            .require_current_reviewed_create(&checked, &selected),
        Err(ChatError::Scope)
    );
    let V2ReviewedCreateCheck::Ready(reopened) = f
        .verifier
        .check_selected_reviewed_create(
            (&pin, membership),
            (&selected, &record),
            &frozen,
            (&ledger, &view),
            &[],
        )
        .unwrap()
    else {
        panic!("a freshly checked historical approval should still match its original");
    };
    let _closing = view.revoker().close();
    assert_eq!(
        f.verifier
            .require_current_reviewed_create(&reopened, &selected),
        Err(ChatError::Scope)
    );
}

#[test]
fn reviewed_create_refuses_private_crossed_and_nonapproval_records() {
    let mut f = Fixture::new();
    let pin = genesis_pin(&f);
    let key = DeviceKeypair::from_seed(&[107; 32]);
    let other = DeviceKeypair::from_seed(&[108; 32]);
    let author = f.add(&key, 30);
    let wrong_author = f.add(&other, 31);
    let create = create(&key, &author, 1);
    let frozen = freeze(&mut f, &author);
    let wrong_frozen = freeze(&mut f, &wrong_author);
    let membership = (f.verifier.replay_data().header().id(), f.epoch());
    let (ledger, view) = f.ledger();
    let evidence = f
        .verifier
        .verify_removed_create_for_review(&create.encode(), &frozen, &ledger, &view, &[])
        .unwrap();
    let held = evidence.encode_record_data(&create.encode()).unwrap();
    let position = ReviewStreamPosition {
        sequence: 1,
        previous: None,
        membership_header: membership.0,
        membership_epoch: membership.1,
    };
    let approval = decision(&evidence, 45, ReviewDecision::Approve, None);
    let (selected, frame) = public_record(&f.owner, &pin, position, &approval, &held);
    let record = ReviewStreamRecord::decode(&pin, &frame).unwrap();
    assert!(f
        .verifier
        .check_selected_reviewed_create(
            (&pin, membership),
            (&selected, &record),
            &wrong_frozen,
            (&ledger, &view),
            &[],
        )
        .is_err());
    assert!(matches!(
        f.verifier.check_selected_reviewed_create(
            (&pin, frozen.removal),
            (&selected, &record),
            &frozen,
            (&ledger, &view),
            &[],
        ),
        Err(ChatError::Scope)
    ));

    let alternate = decision(&evidence, 46, ReviewDecision::Approve, None);
    let (crossed, _) = public_record(&f.owner, &pin, position, &alternate, &held);
    let mut altered = selected.encode();
    *altered.last_mut().unwrap() ^= 1;
    let altered = ReviewStreamEntry::decode(&altered).unwrap();
    for mismatched in [&crossed, &altered] {
        assert!(matches!(
            f.verifier.check_selected_reviewed_create(
                (&pin, membership),
                (mismatched, &record),
                &frozen,
                (&ledger, &view),
                &[],
            ),
            Err(ChatError::Scope)
        ));
    }

    let rejection = decision(&evidence, 47, ReviewDecision::Reject, None);
    let (reject, rejected) = public_record(&f.owner, &pin, position, &rejection, &held);
    let reopen = decision(&evidence, 48, ReviewDecision::Reopen, Some(rejection.id()));
    let successor = ReviewStreamPosition {
        sequence: 2,
        previous: Some(reject.id()),
        ..position
    };
    let (reopen, reopened) = public_record(&f.owner, &pin, successor, &reopen, &held);
    for (entry, raw) in [(&reject, &rejected), (&reopen, &reopened)] {
        let record = ReviewStreamRecord::decode(&pin, raw).unwrap();
        assert!(matches!(
            f.verifier.check_selected_reviewed_create(
                (&pin, membership),
                (entry, &record),
                &frozen,
                (&ledger, &view),
                &[],
            ),
            Err(ChatError::Permission)
        ));
    }

    let private =
        ReviewStreamEntry::sign_using(&pin, position, &approval, |data| Ok(f.owner.key.sign(data)))
            .unwrap();
    let private_frame = ReviewStreamRecord::encode(&pin, &private, &approval, &held).unwrap();
    let private_record = ReviewStreamRecord::decode(&pin, &private_frame).unwrap();
    assert!(matches!(
        f.verifier.check_selected_reviewed_create(
            (&pin, membership),
            (&private, &private_record),
            &frozen,
            (&ledger, &view),
            &[],
        ),
        Err(ChatError::Dependency)
    ));

    let foreign = FounderGenesisAnchorData {
        group: pin.group.clone(),
        state: OwnerStateCommitment::from_bytes([91; 32]),
        ..pin
    };
    let (foreign_entry, foreign_frame) =
        public_record(&f.owner, &foreign, position, &approval, &held);
    let foreign_record = ReviewStreamRecord::decode(&foreign, &foreign_frame).unwrap();
    assert!(matches!(
        f.verifier.check_selected_reviewed_create(
            (&foreign, membership),
            (&foreign_entry, &foreign_record),
            &frozen,
            (&ledger, &view),
            &[],
        ),
        Err(ChatError::Scope)
    ));
    assert_eq!(ledger.state(create.id()), None);
}
