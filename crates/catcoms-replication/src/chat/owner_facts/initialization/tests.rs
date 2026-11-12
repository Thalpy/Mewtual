//! Actual signatures and original codec bytes. Synthetic transition digests are syntax-only
//! inputs; actual MLS/factory consumers separately establish admission and historical authority.
use super::*;
use catcoms_crypto::DeviceKeypair;

fn sign(body: OwnerFactBody) -> SignedOwnerFact {
    let key = DeviceKeypair::from_seed(&[37; 32]);
    let profile =
        (!matches!(body, OwnerFactBody::Profile { .. })).then(|| OwnerFactId::from_bytes([2; 32]));
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
fn subject() -> [u8; 32] {
    DeviceKeypair::from_seed(&[37; 32])
        .verifying_key()
        .to_bytes()
}
fn basis() -> SignedOwnerFact {
    sign(OwnerFactBody::Basis {
        channel: 9,
        admission: OwnerFactId::from_bytes([3; 32]),
        grant: GrantId::from_bytes([4; 32]),
        subject: subject(),
        covered: (0..MAX_BASIS_COVERAGE)
            .map(|index| BasisId::from_bytes([index as u8; 32]))
            .collect(),
    })
}
fn wrap(semantic: &[u8], signature: &[u8; 64]) -> Vec<u8> {
    let mut encoder = Encoder::new();
    encoder.put_bytes(ENVELOPE_DOMAIN).unwrap();
    encoder.put_bytes(semantic).unwrap();
    encoder.put_bytes(signature).unwrap();
    encoder.finish()
}

#[test]
fn initialization_fact_shapes_preserve_all_actual_constructor_kinds_and_do_not_verify_signatures() {
    let facts = [
        sign(OwnerFactBody::Profile {
            creation: [1; 32],
            founder: subject(),
        }),
        sign(OwnerFactBody::Admission {
            subject: subject(),
            origin: AdmissionOrigin::Founder { creation: [1; 32] },
            predecessor: OwnerFactId::from_bytes([2; 32]),
        }),
        sign(OwnerFactBody::Admission {
            subject: subject(),
            origin: AdmissionOrigin::Add {
                commit: [3; 32],
                before_epoch: 7,
                after_epoch: 8,
                key_package: [4; 32],
                credential: [5; 32],
            },
            predecessor: OwnerFactId::from_bytes([2; 32]),
        }),
        sign(OwnerFactBody::RealmGrant {
            channel: 9,
            admission: OwnerFactId::from_bytes([3; 32]),
            subject: subject(),
            rights: Rights::MEMBER,
        }),
        basis(),
        sign(OwnerFactBody::JoinBootstrap {
            key_package: [4; 32],
            subject: subject(),
            welcome: [5; 32],
            expected_founder: subject(),
        }),
    ];
    for fact in facts {
        let bytes = fact.encode();
        assert!(fact.verify());
        SignedOwnerFact::preflight_initialization(&bytes).unwrap();
        assert_eq!(SignedOwnerFact::decode(&bytes).unwrap(), fact);
        let mut bad_signature = bytes;
        *bad_signature.last_mut().unwrap() ^= 1;
        SignedOwnerFact::preflight_initialization(&bad_signature).unwrap();
        assert!(
            !SignedOwnerFact::decode(&bad_signature).unwrap().verify(),
            "a syntax preflight must never be mistaken for verified historical authority"
        );
    }
}

#[test]
fn initialization_fact_basis_count_and_exact_framing_refuse_before_any_owned_fact_decode() {
    let fact = basis();
    let mut semantic = fact.semantic_bytes();
    let offset = {
        let mut decoder = Decoder::new(&semantic);
        assert_eq!(decoder.get_bytes().unwrap(), FACT_DOMAIN);
        assert_eq!(decoder.get_u8().unwrap(), 4);
        decoder.get_bytes().unwrap();
        decoder.get_u16().unwrap();
        get_digest(&mut decoder).unwrap();
        get_digest(&mut decoder).unwrap();
        decoder.get_u128().unwrap();
        for _ in 0..3 {
            get_digest(&mut decoder).unwrap();
        }
        semantic.len() - decoder.remaining()
    };
    semantic[offset..offset + 2].copy_from_slice(&((MAX_BASIS_COVERAGE + 1) as u16).to_be_bytes());
    assert!(matches!(
        SignedOwnerFact::preflight_initialization(&wrap(&semantic, fact.signature())),
        Err(ChatError::Bound)
    ));
    semantic[offset..offset + 2].copy_from_slice(&(MAX_BASIS_COVERAGE as u16).to_be_bytes());
    semantic.truncate(semantic.len() - 36);
    assert!(matches!(
        SignedOwnerFact::preflight_initialization(&wrap(&semantic, fact.signature())),
        Err(ChatError::Malformed)
    ));
    let mut trailing = fact.encode();
    trailing.push(0);
    assert!(SignedOwnerFact::preflight_initialization(&trailing).is_err());
    assert!(matches!(
        SignedOwnerFact::preflight_initialization(&vec![0; MAX_SMALL_FACT_BYTES + 1]),
        Err(ChatError::Bound)
    ));
}

#[test]
fn broader_valid_acceptance_and_removal_evidence_is_unsupported_instead_of_truncated() {
    let facts = [
        sign(OwnerFactBody::Acceptance {
            channel: 9,
            action: ActionId::from_bytes([3; 32]),
            admission: OwnerFactId::from_bytes([4; 32]),
            subject: subject(),
            grant: GrantId::from_bytes([5; 32]),
            basis: BasisId::from_bytes([6; 32]),
            create: Some(ActionId::from_bytes([7; 32])),
        }),
        sign(OwnerFactBody::Removal {
            admission: OwnerFactId::from_bytes([4; 32]),
            subject: subject(),
            commit: [5; 32],
            before_epoch: 7,
            after_epoch: 8,
            accepted: Vec::new(),
        }),
    ];
    for fact in facts {
        let bytes = fact.encode();
        assert!(SignedOwnerFact::decode(&bytes).unwrap().verify());
        assert!(matches!(
            SignedOwnerFact::preflight_initialization(&bytes),
            Err(ChatError::Bound)
        ));
        assert_eq!(fact.encode(), bytes);
    }
}
