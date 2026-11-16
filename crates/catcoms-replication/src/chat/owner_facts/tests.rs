//! Real signature/codec regressions. Synthetic transition digests here are only codec inputs;
//! the separate producer regressions must obtain their observations from actual MLS operations.
use super::*;
use crate::chat::{ActionSpec, Body, Realm, SignedAction};
use catcoms_crypto::DeviceKeypair;

fn key() -> DeviceKeypair {
    DeviceKeypair::from_seed(&[19; 32])
}
fn label(n: u8) -> OwnerFactId {
    OwnerFactId::from_bytes([n; 32])
}
fn sign(body: OwnerFactBody) -> SignedOwnerFact {
    let key = key();
    let profile = (!matches!(body, OwnerFactBody::Profile { .. })).then(|| label(2));
    SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: vec![7; MAX_GROUP_ID_BYTES],
            profile,
            body,
        },
        key.verifying_key().to_bytes(),
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap()
}
fn wrap(semantic: &[u8], signature: &[u8; 64]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(ENVELOPE_DOMAIN).unwrap();
    e.put_bytes(semantic).unwrap();
    e.put_bytes(signature).unwrap();
    e.finish()
}
fn removal(accepted: Vec<ActionId>) -> SignedOwnerFact {
    sign(OwnerFactBody::Removal {
        admission: label(3),
        subject: [4; 32],
        commit: [5; 32],
        before_epoch: 7,
        after_epoch: 8,
        accepted,
    })
}

#[test]
fn owner_fact_all_closed_kinds_round_trip_real_signatures() {
    let subject = key().verifying_key().to_bytes();
    let facts = vec![
        sign(OwnerFactBody::Profile {
            creation: [1; 32],
            founder: subject,
        }),
        sign(OwnerFactBody::Admission {
            subject,
            origin: AdmissionOrigin::Founder { creation: [1; 32] },
            predecessor: label(2),
        }),
        sign(OwnerFactBody::Admission {
            subject,
            origin: AdmissionOrigin::Add {
                commit: [3; 32],
                before_epoch: 8,
                after_epoch: 9,
                key_package: [4; 32],
                credential: [5; 32],
            },
            predecessor: label(2),
        }),
        sign(OwnerFactBody::RealmGrant {
            channel: 9,
            admission: label(3),
            subject,
            rights: Rights::MEMBER,
        }),
        sign(OwnerFactBody::Basis {
            channel: 9,
            admission: label(3),
            grant: GrantId::from_bytes([6; 32]),
            subject,
            covered: (0..MAX_BASIS_COVERAGE)
                .map(|n| BasisId::from_bytes([n as u8; 32]))
                .collect(),
        }),
        sign(OwnerFactBody::Acceptance {
            channel: 9,
            action: ActionId::from_bytes([7; 32]),
            admission: label(3),
            subject,
            grant: GrantId::from_bytes([6; 32]),
            basis: BasisId::from_bytes([8; 32]),
            create: None,
        }),
        sign(OwnerFactBody::Acceptance {
            channel: 9,
            action: ActionId::from_bytes([7; 32]),
            admission: label(3),
            subject,
            grant: GrantId::from_bytes([6; 32]),
            basis: BasisId::from_bytes([8; 32]),
            create: Some(ActionId::from_bytes([9; 32])),
        }),
        removal(vec![]),
        sign(OwnerFactBody::JoinBootstrap {
            key_package: [4; 32],
            subject,
            welcome: [10; 32],
            expected_founder: subject,
        }),
    ];
    for fact in facts {
        assert!(fact.verify());
        let encoded = fact.encode();
        let decoded = SignedOwnerFact::decode(&encoded).unwrap();
        assert_eq!(decoded, fact);
        assert_eq!(decoded.encode(), encoded);
        assert!(decoded.verify());
        assert!(encoded.len() <= fact.maximum_encoded_bytes());
    }
}

#[test]
fn owner_removal_maximum_real_action_set_matches_reviewed_complete_envelope_charge() {
    let key = key();
    let realm = Realm::new(&[7; MAX_GROUP_ID_BYTES], 9).unwrap();
    // These are 4,096 genuinely signed distinct actions, not a short cut padded with labels.
    let mut ids: Vec<_> = (0..MAX_CUT_ACTIONS)
        .map(|n| {
            let mut nonce = [0; 16];
            nonce[..8].copy_from_slice(&(n as u64).to_be_bytes());
            let action = SignedAction::sign(
                &key,
                ActionSpec {
                    realm: realm.clone(),
                    basis: BasisId::from_bytes([8; 32]),
                    grant: GrantId::from_bytes([6; 32]),
                    nonce,
                    presented_ms: 1000,
                    parents: vec![],
                    body: Body::Create {
                        text: format!("retained action {n}"),
                        reply_to: None,
                    },
                },
            )
            .unwrap();
            assert!(action.verify());
            action.id()
        })
        .collect();
    ids.sort();
    assert_eq!(ids.len(), MAX_CUT_ACTIONS);
    assert!(strict_order(&ids));
    let fact = removal(ids);
    assert_eq!(fact.semantic_bytes().len(), 366 + 131_202);
    assert_eq!(fact.encode().len(), 131_680);
    assert!(fact.encode().len() <= MAX_REMOVAL_BYTES);
    assert!(SignedOwnerFact::decode(&fact.encode()).unwrap().verify());
    assert_eq!(
        ADMISSION_RESERVED_BYTES,
        4 * 1024 * 5 + MAX_REMOVAL_BYTES * 2
    );
    assert_eq!(ADMISSION_RESERVED_ROWS, 7);
    assert_eq!(
        MAX_CONTROL_SLOTS,
        MAX_RETAINED_ADMISSIONS * 3 + MAX_OWNER_OPERATIONS * 3
    );
}

#[test]
fn owner_fact_signed_incarnation_changes_grant_and_basis_without_relabeling_old_actions() {
    let subject = [4; 32];
    let admission = |kp| {
        sign(OwnerFactBody::Admission {
            subject,
            predecessor: label(2),
            origin: AdmissionOrigin::Add {
                commit: [3; 32],
                before_epoch: 8,
                after_epoch: 9,
                key_package: [kp; 32],
                credential: [5; 32],
            },
        })
    };
    let a1 = admission(11);
    let a2 = admission(12);
    assert_ne!(a1.id(), a2.id());
    let grant = |admission| {
        sign(OwnerFactBody::RealmGrant {
            channel: 9,
            admission,
            subject,
            rights: Rights::MEMBER,
        })
    };
    let g1 = grant(a1.id());
    let g2 = grant(a2.id());
    assert_ne!(g1.grant_id(), g2.grant_id());
    let basis = |admission, grant| {
        sign(OwnerFactBody::Basis {
            channel: 9,
            admission,
            grant,
            subject,
            covered: vec![],
        })
    };
    assert_ne!(
        basis(a1.id(), g1.grant_id().unwrap()).basis_id(),
        basis(a2.id(), g2.grant_id().unwrap()).basis_id()
    );
    let mut other_channel = g1.spec().clone();
    if let OwnerFactBody::RealmGrant { channel, .. } = &mut other_channel.body {
        *channel = 10;
    }
    let key = key();
    let other =
        SignedOwnerFact::sign_using(other_channel, key.verifying_key().to_bytes(), |bytes| {
            Ok(key.sign(bytes))
        })
        .unwrap();
    assert_ne!(g1.grant_id(), other.grant_id());
    assert_eq!(SignedOwnerFact::decode(&a1.encode()).unwrap().id(), a1.id());
}

#[test]
fn owner_fact_invalid_signature_never_becomes_a_verified_fact() {
    let fact = removal(vec![]);
    let mut wrong_signature = *fact.signature();
    wrong_signature[0] ^= 1;
    let bad = SignedOwnerFact::decode(&wrap(&fact.semantic_bytes(), &wrong_signature)).unwrap();
    assert_eq!(
        bad.id(),
        fact.id(),
        "semantic identity cannot certify a signature variant"
    );
    assert!(!bad.verify());
    assert!(
        fact.verify(),
        "a later good representation remains independently valid"
    );
    let other_key = DeviceKeypair::from_seed(&[20; 32]);
    assert_eq!(
        SignedOwnerFact::sign_using(fact.spec().clone(), *fact.issuer(), |bytes| Ok(
            other_key.sign(bytes)
        )),
        Err(ChatError::Signature)
    );
}

#[test]
fn owner_fact_unknown_grammar_lengths_and_trailing_bytes_refuse() {
    let fact = removal(vec![ActionId::from_bytes([1; 32])]);
    let mut semantic = fact.semantic_bytes();
    semantic[4 + FACT_DOMAIN.len()] = 8;
    assert!(SignedOwnerFact::decode(&wrap(&semantic, fact.signature())).is_err());
    let mut trailing = fact.encode();
    trailing.push(0);
    assert!(SignedOwnerFact::decode(&trailing).is_err());
    let mut inner_tail = fact.semantic_bytes();
    inner_tail.push(0);
    assert!(SignedOwnerFact::decode(&wrap(&inner_tail, fact.signature())).is_err());
    assert_eq!(
        SignedOwnerFact::decode(&vec![0; MAX_REMOVAL_BYTES + 1]),
        Err(ChatError::Bound)
    );
    // Full maximum-group common header plus admission/subject/commit and epoch pair.
    let count_offset = 366 + 36 * 3 + 16;
    let mut bad_count = fact.semantic_bytes();
    bad_count[count_offset..count_offset + 2].copy_from_slice(&u16::MAX.to_be_bytes());
    assert_eq!(
        SignedOwnerFact::decode(&wrap(&bad_count, fact.signature())),
        Err(ChatError::Bound)
    );
    let mut bad_packed_length = fact.semantic_bytes();
    bad_packed_length[count_offset..count_offset + 2].copy_from_slice(&0u16.to_be_bytes());
    assert_eq!(
        SignedOwnerFact::decode(&wrap(&bad_packed_length, fact.signature())),
        Err(ChatError::Malformed)
    );
}

#[test]
fn owner_fact_bounds_order_and_epoch_overflow_refuse_before_signing() {
    let fact = removal(vec![]);
    let key = key();
    let invalid = [
        OwnerFactBody::Removal {
            admission: label(3),
            subject: [4; 32],
            commit: [5; 32],
            before_epoch: u64::MAX,
            after_epoch: 0,
            accepted: vec![],
        },
        OwnerFactBody::Removal {
            admission: label(3),
            subject: [4; 32],
            commit: [5; 32],
            before_epoch: 7,
            after_epoch: 8,
            accepted: vec![ActionId::from_bytes([1; 32]); 2],
        },
        OwnerFactBody::Removal {
            admission: label(3),
            subject: [4; 32],
            commit: [5; 32],
            before_epoch: 7,
            after_epoch: 8,
            accepted: vec![ActionId::from_bytes([1; 32]); MAX_CUT_ACTIONS + 1],
        },
        OwnerFactBody::Basis {
            channel: 9,
            admission: label(3),
            subject: [4; 32],
            grant: GrantId::from_bytes([6; 32]),
            covered: vec![BasisId::from_bytes([8; 32]); MAX_BASIS_COVERAGE + 1],
        },
    ];
    for body in invalid {
        let mut spec = fact.spec().clone();
        spec.body = body;
        assert!(
            SignedOwnerFact::sign_using(spec, key.verifying_key().to_bytes(), |_| panic!(
                "invalid input must never reach key custody"
            ))
            .is_err()
        );
    }
    let mut too_large_group = fact.spec().clone();
    too_large_group.group.push(7);
    assert_eq!(
        SignedOwnerFact::sign_using(too_large_group, *fact.issuer(), |_| panic!(
            "oversized group must never reach key custody"
        )),
        Err(ChatError::Bound)
    );
    let mut absent_profile = fact.spec().clone();
    absent_profile.profile = None;
    assert_eq!(
        SignedOwnerFact::sign_using(absent_profile, *fact.issuer(), |_| panic!(
            "scope mismatch must never reach key custody"
        )),
        Err(ChatError::Scope)
    );
}

#[test]
fn owner_fact_public_types_preserve_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<SignedOwnerFact>();
    send_sync::<OwnerFactSpec>();
}
