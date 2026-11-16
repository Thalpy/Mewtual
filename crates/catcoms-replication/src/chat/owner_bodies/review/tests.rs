//! Signed-data/transition regressions only, not current-governor or UI transaction evidence.
use super::*;
use crate::chat::acceptance_tree::TreeContext;
use crate::chat::owner_header::{HeaderSpec, OwnerStateCommitment, OwnerTenureId};
use crate::chat::{ActionSpec, Body, Realm};
use catcoms_crypto::DeviceKeypair;

fn target(action: &SignedAction) -> AcceptanceRecord {
    AcceptanceRecord::new(AcceptanceSpec {
        group: action.spec().realm.group().to_vec(),
        context: TreeContext {
            profile: OwnerFactId::from_bytes([1; 32]),
            admission: OwnerFactId::from_bytes([2; 32]),
        },
        channel: action.spec().realm.channel(),
        action: action.id(),
        subject: *action.signer(),
        grant: action.spec().grant,
        basis: action.spec().basis,
        create: action.spec().body.create(),
    })
    .unwrap()
}
fn maximum_target() -> (SignedAction, AcceptanceRecord) {
    let key = DeviceKeypair::from_seed(&[41; 32]);
    let create = ActionId::from_bytes([9; 32]);
    let action = SignedAction::sign(
        &key,
        ActionSpec {
            realm: Realm::new(&[7; MAX_GROUP_ID_BYTES], 5).unwrap(),
            basis: BasisId::from_bytes([3; 32]),
            grant: GrantId::from_bytes([4; 32]),
            nonce: [5; 16],
            presented_ms: 100,
            parents: vec![create],
            body: Body::Edit {
                create,
                text: "reviewable edit".into(),
            },
        },
    )
    .unwrap();
    let target = target(&action);
    (action, target)
}
fn decision(kind: ReviewDecision, previous: Option<OwnerBodyId>, nonce: u8) -> ReviewDecisionBody {
    ReviewDecisionBody::new(ReviewSpec {
        target: maximum_target().1,
        intent: [nonce; 16],
        decision: kind,
        previous,
    })
    .unwrap()
}
fn header(body: &ReviewDecisionBody) -> SignedOwnerHeader {
    let key = DeviceKeypair::from_seed(&[42; 32]);
    SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: body.spec().target.spec().group.clone(),
            profile: body.spec().target.spec().context.profile,
            issuer: key.verifying_key().to_bytes(),
            tenure: OwnerTenureId::from_bytes([12; 32]),
            parent: OwnerHeaderId::from_bytes([13; 32]),
            sequence: 2,
            kind: HeaderKind::ReviewDecision,
            body: body.id(),
            state: OwnerStateCommitment::from_bytes([14; 32]),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[test]
fn maximum_frame_and_case_body_domains_match_independent_layout() {
    let body = decision(
        ReviewDecision::Approve,
        Some(OwnerBodyId::from_bytes([8; 32])),
        1,
    );
    assert_eq!(MAX_REVIEW_BODY_BYTES, 665);
    assert_eq!(body.encode().len(), 665);
    assert_eq!(ReviewDecisionBody::decode(&body.encode()).unwrap(), body);
    let mut expected = Vec::new();
    field(&mut expected, b"mewtual/chat/review-decision/v2\0");
    field(&mut expected, &body.spec().target.encode());
    field(&mut expected, &[1; 16]);
    expected.extend_from_slice(&[1, 1]);
    field(&mut expected, &[8; 32]);
    assert_eq!(body.encode(), expected);
    let mut body_id = b"mewtual/chat/owner-body-id/v2\0".to_vec();
    body_id.extend_from_slice(&expected);
    assert_eq!(body.id().as_bytes(), blake3::hash(&body_id).as_bytes());
    let mut case_id = b"mewtual/chat/review-case/v2\0".to_vec();
    case_id.extend_from_slice(&body.spec().target.encode());
    assert_eq!(body.case_id().as_bytes(), blake3::hash(&case_id).as_bytes());
}

#[test]
fn every_valid_transition_and_all_other_pairs_are_closed() {
    let approve = decision(ReviewDecision::Approve, None, 1);
    let reject = decision(ReviewDecision::Reject, None, 2);
    assert_eq!(approve.verify_new_transition_from(None), Ok(()));
    assert_eq!(reject.verify_new_transition_from(None), Ok(()));
    for previous in [
        approve,
        reject,
        decision(
            ReviewDecision::Reopen,
            Some(OwnerBodyId::from_bytes([8; 32])),
            3,
        ),
    ] {
        for next in [
            ReviewDecision::Approve,
            ReviewDecision::Reject,
            ReviewDecision::Reopen,
        ] {
            let current = decision(next, Some(previous.id()), 4);
            let allowed = matches!(
                (previous.spec().decision, next),
                (ReviewDecision::Reject, ReviewDecision::Reopen)
                    | (
                        ReviewDecision::Reopen,
                        ReviewDecision::Approve | ReviewDecision::Reject
                    )
            );
            assert_eq!(
                current.verify_new_transition_from(Some(&previous)).is_ok(),
                allowed
            );
        }
    }
    let reopened = decision(
        ReviewDecision::Reopen,
        Some(OwnerBodyId::from_bytes([8; 32])),
        5,
    );
    assert_eq!(
        reopened.verify_new_transition_from(None),
        Err(ChatError::Dependency)
    );
    let invalid = ReviewSpec {
        target: maximum_target().1,
        intent: [0; 16],
        decision: ReviewDecision::Reopen,
        previous: None,
    };
    assert_eq!(ReviewDecisionBody::new(invalid), Err(ChatError::Dependency));
}

#[test]
fn changed_nonce_decision_and_previous_commit_body_but_preserve_exact_case() {
    let original = decision(ReviewDecision::Approve, None, 1);
    for changed in [
        decision(ReviewDecision::Approve, None, 2),
        decision(ReviewDecision::Reject, None, 1),
        decision(
            ReviewDecision::Approve,
            Some(OwnerBodyId::from_bytes([8; 32])),
            1,
        ),
    ] {
        assert_eq!(changed.case_id(), original.case_id());
        assert_ne!(changed.id(), original.id());
    }
    assert_eq!(
        original.verify_new_transition_from(Some(&original)),
        Err(ChatError::Dependency),
        "exact retry must lookup its retained original outcome, not sign another transition"
    );
}

#[test]
fn wrong_previous_foreign_case_and_omitted_current_head_do_not_match() {
    let previous = decision(ReviewDecision::Reject, None, 1);
    let current = decision(ReviewDecision::Reopen, Some(previous.id()), 2);
    assert_eq!(current.verify_new_transition_from(Some(&previous)), Ok(()));
    let wrong = decision(
        ReviewDecision::Reopen,
        Some(OwnerBodyId::from_bytes([15; 32])),
        2,
    );
    assert_eq!(
        wrong.verify_new_transition_from(Some(&previous)),
        Err(ChatError::Dependency)
    );
    let mut spec = current.spec().clone();
    let mut record = spec.target.spec().clone();
    record.channel += 1;
    spec.target = AcceptanceRecord::new(record).unwrap();
    let foreign = ReviewDecisionBody::new(spec).unwrap();
    assert_ne!(foreign.case_id(), previous.case_id());
    assert_eq!(
        foreign.verify_new_transition_from(Some(&previous)),
        Err(ChatError::Dependency)
    );
    let missing_previous = decision(ReviewDecision::Approve, None, 3);
    assert_eq!(
        missing_previous.verify_new_transition_from(Some(&previous)),
        Err(ChatError::Dependency)
    );
    let reopened = decision(ReviewDecision::Reopen, Some(previous.id()), 4);
    let finalized = decision(ReviewDecision::Approve, Some(reopened.id()), 5);
    assert_eq!(
        current.verify_new_transition_from(Some(&finalized)),
        Err(ChatError::Dependency)
    );
    // Current-state lookup is still mandatory: supplying the genuinely old Reject passes this
    // DATA relation but cannot prove it remains the selected CURRENT case after finalized.
    assert_eq!(current.verify_new_transition_from(Some(&previous)), Ok(()));
}

#[test]
fn actual_action_and_signed_header_binding_do_not_grant_reviewed_permission_alone() {
    let (action, target) = maximum_target();
    let body = ReviewDecisionBody::new(ReviewSpec {
        target,
        intent: [1; 16],
        decision: ReviewDecision::Approve,
        previous: None,
    })
    .unwrap();
    assert_eq!(body.matches_action(&action), Ok(()));
    let signed = header(&body);
    assert_eq!(body.verify_header_binding(&signed), Ok(()));
    let key = DeviceKeypair::from_seed(&[42; 32]);
    for field in 0..4 {
        let mut changed = signed.spec().clone();
        match field {
            0 => changed.kind = HeaderKind::AcceptanceAppend,
            1 => changed.group[0] ^= 1,
            2 => changed.profile = OwnerFactId::from_bytes([16; 32]),
            _ => changed.body = OwnerBodyId::from_bytes([17; 32]),
        }
        let changed = SignedOwnerHeader::sign_using(changed, |bytes| Ok(key.sign(bytes))).unwrap();
        assert_eq!(body.verify_header_binding(&changed), Err(ChatError::Scope));
    }
    let mut raw = signed.encode();
    *raw.last_mut().unwrap() ^= 1;
    assert_eq!(
        body.verify_header_binding(&SignedOwnerHeader::decode(&raw).unwrap()),
        Err(ChatError::Signature)
    );
    let mut changed = action.spec().clone();
    changed.nonce[0] ^= 1;
    let changed = SignedAction::sign(&DeviceKeypair::from_seed(&[41; 32]), changed).unwrap();
    assert_eq!(body.matches_action(&changed), Err(ChatError::Scope));
    let mut raw = action.encode();
    *raw.last_mut().unwrap() ^= 1;
    assert_eq!(
        body.matches_action(&SignedAction::decode(&raw).unwrap()),
        Err(ChatError::Signature)
    );
    // No selected owner root, current-case proof, frozen-cut inclusion or opaque admission
    // token has been supplied by these pure signature/data checks.
}

#[test]
fn every_truncated_outer_frame_and_wrong_option_nonce_or_previous_length_refuses() {
    let body = decision(
        ReviewDecision::Approve,
        Some(OwnerBodyId::from_bytes([8; 32])),
        1,
    );
    let bytes = body.encode();
    for end in 0..bytes.len() {
        assert!(ReviewDecisionBody::decode(&bytes[..end]).is_err());
    }
    let target_start = 4 + DOMAIN.len();
    let nonce_start = target_start + 4 + MAX_ACCEPTANCE_RECORD_BYTES;
    let decision_at = nonce_start + 4 + 16;
    let previous_at = decision_at + 2;
    for length in [0u32, 15, 17, u32::MAX] {
        let mut bad = bytes.clone();
        bad[nonce_start..nonce_start + 4].copy_from_slice(&length.to_be_bytes());
        assert!(ReviewDecisionBody::decode(&bad).is_err());
    }
    for length in [0u32, 31, 33, u32::MAX] {
        let mut bad = bytes.clone();
        bad[previous_at..previous_at + 4].copy_from_slice(&length.to_be_bytes());
        assert!(ReviewDecisionBody::decode(&bad).is_err());
    }
    for tag in [0, 4, 255] {
        let mut bad = bytes.clone();
        bad[decision_at] = tag;
        assert_eq!(ReviewDecisionBody::decode(&bad), Err(ChatError::Malformed));
    }
    let mut unknown = bytes.clone();
    unknown[decision_at + 1] = 2;
    assert_eq!(
        ReviewDecisionBody::decode(&unknown),
        Err(ChatError::Malformed)
    );
    let mut omitted = bytes.clone();
    omitted[decision_at + 1] = 0;
    assert!(ReviewDecisionBody::decode(&omitted).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(ReviewDecisionBody::decode(&trailing).is_err());
    let mut domain = bytes;
    domain[4] ^= 1;
    assert_eq!(
        ReviewDecisionBody::decode(&domain),
        Err(ChatError::Malformed)
    );
}

#[test]
fn malformed_or_oversized_target_and_reopen_without_previous_refuse() {
    let valid = decision(ReviewDecision::Approve, None, 1);
    let start = 4 + DOMAIN.len();
    let mut invalid = valid.encode();
    invalid[start + 4 + 4] ^= 1; // target's domain payload, not outer
    assert!(ReviewDecisionBody::decode(&invalid).is_err());
    let mut extra = Vec::new();
    field(&mut extra, DOMAIN);
    field(&mut extra, &vec![0; MAX_ACCEPTANCE_RECORD_BYTES + 1]);
    field(&mut extra, &[0; 16]);
    extra.extend_from_slice(&[1, 0]);
    assert_eq!(ReviewDecisionBody::decode(&extra), Err(ChatError::Bound));
    let mut reopen = valid.encode();
    let decision_at = reopen.len() - 2;
    reopen[decision_at] = 3;
    assert_eq!(
        ReviewDecisionBody::decode(&reopen),
        Err(ChatError::Dependency)
    );
}

#[test]
fn genuine_action_signature_variants_are_one_case_and_one_review_body() {
    fn unhex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/signature-variants.json")).unwrap();
    let actions: Vec<_> = fixture["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| SignedAction::decode(&unhex(value.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(actions.len(), 2);
    assert!(actions.iter().all(SignedAction::verify));
    assert_ne!(actions[0].signature(), actions[1].signature());
    let bodies: Vec<_> = actions
        .iter()
        .map(|action| {
            ReviewDecisionBody::new(ReviewSpec {
                target: target(action),
                intent: [1; 16],
                decision: ReviewDecision::Approve,
                previous: None,
            })
            .unwrap()
        })
        .collect();
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[0].id(), bodies[1].id());
    assert_eq!(bodies[0].case_id(), bodies[1].case_id());
    assert_eq!(bodies[0].matches_action(&actions[1]), Ok(()));
}

#[test]
fn other_v2_body_kinds_and_legacy_owner_facts_cannot_be_review_decisions() {
    use crate::chat::owner_facts::{OwnerFactBody, OwnerFactSpec, SignedOwnerFact};
    let key = DeviceKeypair::from_seed(&[42; 32]);
    let legacy = SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: vec![7],
            profile: None,
            body: OwnerFactBody::Profile {
                creation: [1; 32],
                founder: key.verifying_key().to_bytes(),
            },
        },
        key.verifying_key().to_bytes(),
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
    .encode();
    assert!(ReviewDecisionBody::decode(&legacy).is_err());
    assert!(ReviewDecisionBody::decode(&maximum_target().1.encode()).is_err());
    let bytes = decision(ReviewDecision::Approve, None, 1).encode();
    assert!(SignedOwnerFact::decode(&bytes).is_err());
    assert!(AcceptanceRecord::decode(&bytes).is_err());
    assert!(AppendBody::decode(&bytes).is_err());
    assert!(RemovalBody::decode(&bytes).is_err());
}
