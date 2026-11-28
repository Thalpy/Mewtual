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

// This independently signed Ed25519 representation is for the exact
// deterministic "A withheld reply" fixture. Test data never leaves Core.
pub(super) fn alternate_withheld_reply(original: &SignedAction) -> SignedAction {
    let signature = [
        0xe2, 0xfe, 0x2a, 0x39, 0xb7, 0x32, 0x66, 0x42, 0x82, 0x7d, 0x6d, 0x82, 0xd4, 0x97, 0x62,
        0x2c, 0xd6, 0xc2, 0x97, 0xfe, 0x78, 0x27, 0x0e, 0xaa, 0x43, 0x6b, 0x01, 0xf0, 0x86, 0xb6,
        0xf3, 0xcb, 0x5b, 0x06, 0xd3, 0xcd, 0xaf, 0xa5, 0x77, 0xbf, 0xfa, 0xc0, 0x54, 0x06, 0xf9,
        0x5e, 0xa2, 0xee, 0x94, 0x30, 0x7b, 0x83, 0x8b, 0x9f, 0xe6, 0x36, 0xb4, 0x2b, 0x08, 0x62,
        0xbb, 0x2d, 0xc7, 0x07,
    ];
    let mut alternative = original.encode();
    let offset = alternative.len() - signature.len();
    alternative[offset..].copy_from_slice(&signature);
    let alternative = SignedAction::decode(&alternative).unwrap();
    assert!(alternative.verify());
    assert_eq!(original.id(), alternative.id());
    assert_ne!(original.signature(), alternative.signature());
    alternative
}

fn archive_view(fixture: &Fixture, ledger: &mut ChatLedger) -> ProjectionView {
    let mut view = ledger
        .begin_local_archive_projection_view_in(&fixture.group_view)
        .unwrap();
    assert!(view.try_ready());
    view
}

fn approval_for(
    fixture: &Fixture,
    pin: &FounderGenesisAnchorData,
    evidence: &V2RemovedCreateEvidence,
    original: &SignedAction,
) -> (ReviewStreamEntry, Zeroizing<Vec<u8>>) {
    public_record(
        &fixture.owner,
        pin,
        ReviewStreamPosition {
            sequence: 1,
            previous: None,
            membership_header: fixture.verifier.replay_data().header().id(),
            membership_epoch: fixture.epoch(),
        },
        &decision(evidence, 45, ReviewDecision::Approve, None),
        &evidence.encode_record_data(&original.encode()).unwrap(),
    )
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

#[test]
fn reviewed_projection_keeps_the_exact_signature_and_original_origin() {
    let mut f = Fixture::new();
    let pin = genesis_pin(&f);
    let key = DeviceKeypair::from_seed(&[109; 32]);
    let other = DeviceKeypair::from_seed(&[110; 32]);
    let author = f.add(&key, 20);
    let parent_author = f.add(&other, 22);
    let parent = create(&other, &parent_author, 1);
    let reply = action(
        &key,
        &author,
        2,
        Body::Create {
            text: "A withheld reply".into(),
            reply_to: Some(parent.id()),
        },
        vec![parent.id()],
    );
    let alternative = alternate_withheld_reply(&reply);
    let (lower, approved) = if reply.signature() < alternative.signature() {
        (reply, alternative)
    } else {
        (alternative, reply)
    };
    let frozen = freeze(&mut f, &author);
    let (mut ledger, live) = f.ledger();
    let view = archive_view(&f, &mut ledger);
    let evidence = f
        .verifier
        .verify_removed_create_for_review(&approved.encode(), &frozen, &ledger, &view, &[])
        .unwrap();
    let (selected, raw) = approval_for(&f, &pin, &evidence, &approved);
    let record = ReviewStreamRecord::decode_public_approval(&pin, selected.id(), &raw).unwrap();
    let scope = (&pin, (f.verifier.replay_data().header().id(), f.epoch()));
    let approval = (&selected, &record);
    let retired = [&frozen];
    let mut crossed = selected.encode();
    *crossed.last_mut().unwrap() ^= 1;
    let crossed = ReviewStreamEntry::decode(&crossed).unwrap();
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            (&crossed, &record),
            &frozen,
            &retired,
            128,
        ),
        Err(ChatError::Scope)
    );
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &live),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Err(ChatError::Scope)
    );
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Ok(V2OrdinaryReceiveStatus::WaitingForDependencies(vec![
            parent.id()
        ]))
    );
    assert!(ledger.action(approved.id()).is_none());
    let retain_parent = |ledger: &mut ChatLedger, view: &ProjectionView| {
        assert_eq!(
            f.verifier.replay_local_archive_dependency(
                ledger,
                view,
                &parent,
                parent_author.admission_data().admission_id(),
                &retired,
                128,
            ),
            Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
        );
    };
    retain_parent(&mut ledger, &view);
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    let result = ledger.guarded_projection(approved.id(), 128).unwrap();
    assert!(result
        .with_publication(|projection| matches!(
            projection,
            GuardedProjection::Visible {
                create, author, reply_to: Some(target), text, certainty: ProjectionCertainty::Proven, ..
            } if *create == approved.id() && author == approved.signer()
                && *target == parent.id() && text == "A withheld reply"
        ))
        .unwrap());
    assert_eq!(
        ledger.import(lower.clone()),
        Err(ChatError::AuthorityConflict)
    );
    assert_eq!(ledger.action(approved.id()), Some(&approved));
    assert_eq!(
        ledger.import(approved.clone()),
        Ok(ImportOutcome::Duplicate(approved.id()))
    );
    assert!(result.with_publication(|_| ()).is_ok());

    // If ordinary storage already selected this semantic action, review
    // neither replaces its signed representative nor promotes certainty.
    let (mut ordinary, _live) = f.ledger();
    let ordinary_view = archive_view(&f, &mut ordinary);
    for (action, admission) in [
        (&parent, parent_author.admission_data().admission_id()),
        (&approved, author.admission_data().admission_id()),
    ] {
        let original = V2LocalArchiveOriginal::from_trusted_local_selection(
            action,
            admission,
            &action.encode(),
        )
        .unwrap();
        assert_eq!(
            f.verifier.project_local_archive_original(
                &mut ordinary,
                &ordinary_view,
                original,
                &retired,
                128
            ),
            Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
        );
    }
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ordinary, &ordinary_view),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Err(ChatError::AuthorityConflict)
    );
    assert!(ordinary
        .guarded_projection(approved.id(), 128)
        .unwrap()
        .with_publication(|projection| matches!(
            projection,
            GuardedProjection::Visible {
                certainty: ProjectionCertainty::Provisional,
                ..
            }
        ),)
        .unwrap());

    // While the source remains live, replacing the original archive
    // view restores the ordinary ledger's signature merge behavior.
    let mut ordinary_replacement = ledger.begin_projection_view_in(&f.group_view).unwrap();
    assert!(ordinary_replacement.try_ready());
    assert_eq!(
        ledger.import(lower.clone()),
        Ok(ImportOutcome::Duplicate(approved.id()))
    );
    assert_eq!(ledger.action(approved.id()), Some(&lower));
    assert_eq!(result.with_publication(|_| ()), Err(ChatError::Scope));

    // Independently check source revocation while the new detached view
    // and cached body still exist.
    let (mut source_replay, _live) = f.ledger();
    let source_view = archive_view(&f, &mut source_replay);
    retain_parent(&mut source_replay, &source_view);
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut source_replay, &source_view),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    let source_result = source_replay
        .guarded_projection(approved.id(), 128)
        .unwrap();
    assert!(source_result.with_publication(|_| ()).is_ok());
    f.negative.0.store(false, Ordering::Release);
    assert_eq!(
        source_result.with_publication(|_| ()),
        Err(ChatError::Scope)
    );
}

#[test]
fn reviewed_projection_requires_retired_parents_from_selected_archive_and_grants_no_mutation() {
    let mut f = Fixture::new();
    let pin = genesis_pin(&f);
    let key = DeviceKeypair::from_seed(&[117; 32]);
    let other = DeviceKeypair::from_seed(&[118; 32]);
    let author = f.add(&key, 31);
    let parent_author = f.add(&other, 32);
    let parent = create(&other, &parent_author, 1);
    let reply = action(
        &key,
        &author,
        2,
        Body::Create {
            text: "Reply to retained history".into(),
            reply_to: Some(parent.id()),
        },
        vec![parent.id()],
    );
    let delete = action(
        &key,
        &author,
        3,
        Body::Delete { create: reply.id() },
        vec![reply.id()],
    );
    let next_reply = action(
        &other,
        &parent_author,
        4,
        Body::Create {
            text: "Reply to reviewed history".into(),
            reply_to: Some(reply.id()),
        },
        vec![reply.id()],
    );
    let frozen = freeze(&mut f, &author);
    let frozen_parent = freeze(&mut f, &parent_author);
    let retired = [&frozen, &frozen_parent];
    let (mut ledger, _live) = f.ledger();
    let view = archive_view(&f, &mut ledger);
    let parent_admission = parent_author.admission_data().admission_id();
    let evidence = f
        .verifier
        .verify_removed_create_for_review(&reply.encode(), &frozen, &ledger, &view, &retired)
        .unwrap();
    let (selected, raw) = approval_for(&f, &pin, &evidence, &reply);
    let record = ReviewStreamRecord::decode_public_approval(&pin, selected.id(), &raw).unwrap();
    let scope = (&pin, (f.verifier.replay_data().header().id(), f.epoch()));
    let approval = (&selected, &record);
    assert_eq!(
        f.verifier.replay_local_archive_dependency(
            &mut ledger,
            &view,
            &parent,
            parent_admission,
            &retired,
            128,
        ),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Ok(V2OrdinaryReceiveStatus::WaitingForDependencies(vec![
            parent.id()
        ]))
    );
    let exact = V2LocalArchiveOriginal::from_trusted_local_selection(
        &parent,
        parent_admission,
        &parent.encode(),
    )
    .unwrap();
    assert_eq!(
        f.verifier
            .project_local_archive_original(&mut ledger, &view, exact, &retired, 128),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    // Mark provenance before any semantic progress. A later step can
    // make the exact original Valid without creating a display token.
    assert!(matches!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            approval,
            &frozen,
            &retired,
            0,
        ),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Pending(_)))
    ));
    ledger.advance(128);
    assert_eq!(ledger.state(reply.id()), Some(ActionState::Valid));
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            approval,
            &frozen,
            &retired,
            0,
        ),
        Err(ChatError::WorkLimit)
    );
    assert!(ledger
        .guarded_projection(reply.id(), 128)
        .unwrap()
        .with_publication(|projection| matches!(projection, GuardedProjection::Held { .. }),)
        .unwrap());
    let mutation = || {
        V2LocalArchiveOriginal::from_trusted_local_selection(
            &delete,
            author.admission_data().admission_id(),
            &delete.encode(),
        )
        .unwrap()
    };
    assert_eq!(
        f.verifier
            .project_local_archive_original(&mut ledger, &view, mutation(), &retired, 128),
        Err(ChatError::Permission)
    );
    assert_eq!(
        f.verifier.project_selected_reviewed_create(
            (&mut ledger, &view),
            scope,
            approval,
            &frozen,
            &retired,
            128,
        ),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    assert_eq!(
        f.verifier
            .verify_removed_create_for_review(
                &next_reply.encode(),
                &frozen_parent,
                &ledger,
                &view,
                &retired
            )
            .unwrap()
            .waiting_for_reply_parent_data(),
        Some(reply.id())
    );
    assert!(!f
        .verifier
        .locally_usable_ordinary_dependency(&ledger, reply.id()));
    assert_eq!(
        f.verifier
            .project_local_archive_original(&mut ledger, &view, mutation(), &retired, 128),
        Err(ChatError::Permission)
    );
    assert!(ledger.action(delete.id()).is_none());
    let guarded = ledger.guarded_projection(reply.id(), 128).unwrap();
    assert!(guarded.with_publication(|_| ()).is_ok());
    let _closing = view.revoker().close();
    assert_eq!(guarded.with_publication(|_| ()), Err(ChatError::Scope));
}
