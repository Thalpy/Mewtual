use super::*;
use crate::chat::cold_actions::OriginalActionDigest;
use crate::chat::owner_header::{OwnerStateCommitment, OwnerTenureId};
use catcoms_crypto::DeviceKeypair;

fn decision(
    action: u8,
    original: u8,
    (choice, previous): (ReviewDecision, Option<OwnerBodyId>),
) -> ExactReviewDecisionBody {
    let target = AcceptanceRecord::new(AcceptanceSpec {
        group: vec![1; 32],
        context: TreeContext {
            profile: OwnerFactId::from_bytes([2; 32]),
            admission: OwnerFactId::from_bytes([3; 32]),
        },
        channel: 4,
        action: ActionId::from_bytes([action; 32]),
        subject: DeviceKeypair::from_seed(&[5; 32])
            .verifying_key()
            .to_bytes(),
        grant: GrantId::from_bytes([6; 32]),
        basis: BasisId::from_bytes([7; 32]),
        create: None,
    })
    .unwrap();
    let body = ReviewDecisionBody::new(ReviewSpec {
        target,
        intent: [8; 16],
        decision: choice,
        previous,
    })
    .unwrap();
    ExactReviewDecisionBody::new(
        body,
        ExactReviewOriginal {
            digest: OriginalActionDigest::from_bytes([original; 32]),
            remove_header: OwnerHeaderId::from_bytes([9; 32]),
            remove_epoch: 3,
        },
    )
    .unwrap()
}

#[test]
fn public_review_stream_replays_redacted_cases_with_their_exact_predecessor() {
    use ReviewDecision::{Approve, Reject, Reopen};
    let founder = DeviceKeypair::from_seed(&[10; 32]);
    let pin = FounderGenesisAnchorData {
        group: vec![1; 32],
        founder: founder.verifying_key().to_bytes(),
        profile: OwnerFactId::from_bytes([2; 32]),
        admission: OwnerFactId::from_bytes([11; 32]),
        tenure: OwnerTenureId::from_bytes([12; 32]),
        header: OwnerHeaderId::from_bytes([13; 32]),
        state: OwnerStateCommitment::from_bytes([14; 32]),
    };
    let first = ReviewStreamPosition {
        sequence: 1,
        previous: None,
        membership_header: OwnerHeaderId::from_bytes([15; 32]),
        membership_epoch: 4,
    };
    let next = |entry: &ReviewStreamEntry| ReviewStreamPosition {
        sequence: entry.position_data().sequence + 1,
        previous: Some(entry.id()),
        ..first
    };
    let signed = |position, body: &ExactReviewDecisionBody| {
        let preimage = UnsignedReviewStreamEntry::prepare_public(&pin, position, body).unwrap();
        let exact = preimage.signing_bytes();
        assert_eq!(exact.len(), MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES);
        assert_eq!(
            UnsignedReviewStreamEntry::read_position_data(exact).unwrap(),
            position
        );
        let raw = founder.sign(exact);
        UnsignedReviewStreamEntry::rederive(&pin, position, body, exact)
            .unwrap()
            .attach_signature(&pin, raw)
            .unwrap()
    };

    let rejected = decision(16, 17, (Reject, None));
    let head = signed(first, &rejected);
    // Peers can verify this sequence without receiving the rejected body.
    let header = head.encode();
    let head = ReviewStreamEntry::decode(&header).unwrap();
    let rejected_step = head.verify_public_next_for_pin(&pin, None, None).unwrap();
    let legacy =
        ReviewStreamEntry::sign_using(&pin, first, &rejected, |bytes| Ok(founder.sign(bytes)))
            .unwrap();
    assert_ne!(legacy.id(), head.id());
    assert_eq!(legacy.encode().len(), REVIEW_STREAM_ENTRY_BYTES);
    assert_eq!(
        legacy.verify_public_next_for_pin(&pin, None, None),
        Err(ChatError::Dependency)
    );
    let legacy_step = signed(next(&legacy), &decision(18, 19, (Approve, None)));
    legacy_step
        .verify_next_for_pin(&pin, Some(&legacy))
        .unwrap();
    assert_eq!(
        legacy_step.verify_public_next_for_pin(&pin, Some(&legacy), None),
        Err(ChatError::Dependency)
    );

    let another = decision(18, 19, (Approve, None));
    let later = signed(next(&head), &another);
    let other_step = later
        .verify_public_next_for_pin(&pin, Some(&head), None)
        .unwrap();
    let same_case = Some((head.decision_id_data(), rejected_step));
    let reopen = decision(16, 17, (Reopen, Some(head.decision_id_data())));
    let entry = signed(next(&later), &reopen);
    assert_eq!(
        entry.verify_public_next_for_pin(&pin, Some(&later), None),
        Err(ChatError::Dependency)
    );
    assert_eq!(
        entry.verify_public_next_for_pin(&pin, Some(&head), same_case),
        Err(ChatError::Dependency)
    );
    assert_eq!(
        entry.verify_public_next_for_pin(
            &pin,
            Some(&later),
            Some((head.decision_id_data(), other_step))
        ),
        Err(ChatError::Scope)
    );
    let changed = decision(16, 20, (Reopen, Some(head.decision_id_data())));
    assert_eq!(
        signed(next(&later), &changed).verify_public_next_for_pin(&pin, Some(&later), same_case),
        Err(ChatError::Scope)
    );
    let reopened_step = entry
        .verify_public_next_for_pin(&pin, Some(&later), same_case)
        .unwrap();

    let approved = decision(16, 17, (Approve, Some(entry.decision_id_data())));
    let approval = signed(next(&entry), &approved);
    assert_eq!(
        approval.verify_public_next_for_pin(&pin, Some(&entry), same_case),
        Err(ChatError::Dependency)
    );
    let approved_step = approval
        .verify_public_next_for_pin(
            &pin,
            Some(&entry),
            Some((entry.decision_id_data(), reopened_step)),
        )
        .unwrap();
    approval.verify_decision_for_pin(&pin, &approved).unwrap();
    let after = decision(16, 17, (Reopen, Some(approval.decision_id_data())));
    assert!(signed(next(&approval), &after)
        .verify_public_next_for_pin(
            &pin,
            Some(&approval),
            Some((approval.decision_id_data(), approved_step))
        )
        .is_err());

    let mut changed_choice = header;
    changed_choice[MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES - 1] = Approve.tag();
    let changed_choice = ReviewStreamEntry::decode(&changed_choice).unwrap();
    assert_eq!(
        changed_choice.verify_for_pin(&pin),
        Err(ChatError::Signature)
    );
}
