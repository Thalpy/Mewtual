use super::*;

fn fixture() -> Agreement {
    Agreement {
        scope: Scope {
            group: [3; 32],
            policy: Some([4; 32]),
            room: u128::MAX,
        },
        epoch: 7,
        low: Endpoint {
            device: DeviceId::from_bytes([1; 32]),
            join: [5; 32],
            key_ref: [6; 32],
            incarnation: [0; 32],
            generation: 1,
        },
        high: Endpoint {
            device: DeviceId::from_bytes([2; 32]),
            join: [7; 32],
            key_ref: [8; 32],
            incarnation: [9; 32],
            generation: u64::MAX,
        },
        low_nonce: [0; 32],
        high_nonce: [11; 32],
        request_digest: [12; 32],
        grant_digest: [13; 32],
        commit_by_wall: 14,
    }
}

// Independent field-by-field vector, not RawOutput or Agreement::encode.
fn vector(a: &Agreement) -> Vec<u8> {
    let mut bytes = vec![1, 1, 2, 1];
    bytes.extend(a.scope.group);
    bytes.push(u8::from(a.scope.policy.is_some()));
    bytes.extend(a.scope.policy.unwrap_or([0; 32]));
    bytes.extend(a.scope.room.to_be_bytes());
    bytes.extend(a.epoch.to_be_bytes());
    bytes.extend(a.low.device.as_bytes());
    bytes.extend(a.high.device.as_bytes());
    for field in [
        a.low.join,
        a.high.join,
        a.low.key_ref,
        a.high.key_ref,
        a.low.incarnation,
        a.high.incarnation,
    ] {
        bytes.extend(field);
    }
    bytes.extend(a.low.generation.to_be_bytes());
    bytes.extend(a.high.generation.to_be_bytes());
    for field in [a.low_nonce, a.high_nonce, a.request_digest, a.grant_digest] {
        bytes.extend(field);
    }
    bytes.extend(a.commit_by_wall.to_be_bytes());
    bytes.extend(1u16.to_be_bytes());
    bytes
}

fn keys() -> (Agreement, EndpointKeyBytes, EndpointKeyBytes) {
    let mut a = fixture();
    let mut first = EndpointKeyBytes {
        ed25519: [20; 32],
        point: [21; 65],
    };
    let mut second = EndpointKeyBytes {
        ed25519: [22; 32],
        point: [23; 65],
    };
    first.point[0] = 4;
    second.point[0] = 4;
    let first_id = DeviceId::from_public_key_bytes(&first.ed25519);
    let second_id = DeviceId::from_public_key_bytes(&second.ed25519);
    let (low, high) = if first_id < second_id {
        (first, second)
    } else {
        (second, first)
    };
    a.low.device = DeviceId::from_public_key_bytes(&low.ed25519);
    a.high.device = DeviceId::from_public_key_bytes(&high.ed25519);
    a.low.key_ref = key_reference(&low.point);
    a.high.key_ref = key_reference(&high.point);
    (a, low, high)
}

#[test]
fn fixed_agreement_vectors_preserve_all_policy_forms_and_present_zero_randomness() {
    for policy in [None, Some([0; 32]), Some([255; 32])] {
        let mut a = fixture();
        a.scope.policy = policy;
        let bytes = a.encode().unwrap();
        assert_eq!(bytes.len(), 503);
        assert_eq!(bytes.as_slice(), vector(&a));
        assert_eq!(Agreement::decode(&bytes).unwrap(), a);
        assert_eq!(bytes[36], u8::from(policy.is_some()));
        assert_eq!(&bytes[37..69], &policy.unwrap_or([0; 32]));
        assert_eq!(a.low.incarnation, [0; 32]);
        assert_eq!(a.low_nonce, [0; 32]);
        assert_eq!(a.high.generation, u64::MAX);
    }
}

#[test]
fn agreement_refuses_all_truncation_trailing_unknown_and_noncanonical_absence() {
    let mut a = fixture();
    a.scope.policy = None;
    let bytes = a.encode().unwrap();
    for end in 0..bytes.len() {
        assert!(Agreement::decode(&bytes[..end]).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(Agreement::decode(&trailing).is_err());
    for offset in [0, 1, 2, 3, 36, 37, 501, 502] {
        let mut bad = bytes;
        bad[offset] ^= 2;
        assert!(Agreement::decode(&bad).is_err(), "closed field {offset}");
    }
    for start in [349, 357] {
        let mut bad = bytes;
        bad[start..start + 8].fill(0);
        assert!(Agreement::decode(&bad).is_err());
    }
    let mut bad = bytes;
    bad[93..125].fill(255);
    assert!(Agreement::decode(&bad).is_err());
}

#[test]
fn round_digest_covers_every_canonical_agreement_byte_and_separates_absent_policy() {
    let a = fixture();
    let bytes = a.encode().unwrap();
    let round = RoundRef::derive(&a).unwrap();
    let mut independently_framed = ROUND_DOMAIN.to_vec();
    independently_framed.extend(vector(&a));
    assert_eq!(
        round.bytes(),
        blake3::hash(&independently_framed).as_bytes()
    );
    for offset in 0..bytes.len() {
        let mut changed = bytes;
        changed[offset] ^= 1;
        if let Ok(other) = Agreement::decode(&changed) {
            assert_ne!(
                round,
                RoundRef::derive(&other).unwrap(),
                "agreement byte {offset}"
            );
        }
    }
    let mut absent = a.clone();
    absent.scope.policy = None;
    let mut present_zero = a.clone();
    present_zero.scope.policy = Some([0; 32]);
    assert_ne!(
        RoundRef::derive(&absent).unwrap(),
        RoundRef::derive(&present_zero).unwrap()
    );
}

#[test]
fn transcript_has_exact_new_domain_agreement_round_and_endpoint_key_vectors() {
    let (a, low, high) = keys();
    let transcript = Transcript::prepare(&a, &low, &high).unwrap();
    assert_eq!(transcript.0.len(), 789);
    let mut expected = Encoder::new();
    expected.put_str("mewtual/voice/pair/admitted/v1").unwrap();
    expected.put_u8(1);
    expected.put_u8(1);
    expected
        .put_bytes(RoundRef::derive(&a).unwrap().bytes())
        .unwrap();
    expected.put_bytes(&vector(&a)).unwrap();
    for endpoint in [&low, &high] {
        expected.put_bytes(&endpoint.ed25519).unwrap();
        expected.put_bytes(&endpoint.point).unwrap();
    }
    assert_eq!(transcript.0.as_slice(), expected.finish());
    assert_eq!(Transcript::decode(&transcript.0).unwrap().0, transcript.0);
    for end in 0..789 {
        assert!(Transcript::decode(&transcript.0[..end]).is_err());
    }
    let mut trailing = transcript.0.to_vec();
    trailing.push(0);
    assert!(Transcript::decode(&trailing).is_err());
    for offset in [4, 34, 35, 40, 76, 583, 619, 688, 724] {
        let mut bad = transcript.0;
        bad[offset] ^= 1;
        assert!(
            Transcript::decode(&bad).is_err(),
            "transcript byte {offset}"
        );
    }
    assert!(Transcript::prepare(&a, &high, &low).is_err());
}

#[test]
fn kdf_salt_and_direction_inputs_are_fixed_separate_domains_without_deriving_keys() {
    let (a, low, high) = keys();
    let t = Transcript::prepare(&a, &low, &high).unwrap();
    assert_eq!(t.salt_input().len(), 830);
    assert_eq!(
        &t.salt_input()[..41],
        b"mewtual/voice/v2/admission/v1/transcript\0"
    );
    assert_eq!(&t.salt_input()[41..], &t.0);
    assert_eq!(t.salt(), <[u8; 32]>::from(Sha256::digest(t.salt_input())));
    let low_info = t.info(Direction::LowToHigh);
    let high_info = t.info(Direction::HighToLow);
    assert_eq!(low_info.len(), 839);
    assert_eq!(high_info.len(), 839);
    assert_eq!(
        &low_info[..50],
        b"mewtual/voice/v2/admission/v1/aes-gcm/low-to-high\0"
    );
    assert_eq!(
        &high_info[..50],
        b"mewtual/voice/v2/admission/v1/aes-gcm/high-to-low\0"
    );
    assert_eq!(&low_info[50..], &t.0);
    assert_eq!(&high_info[50..], &t.0);
    assert_ne!(low_info, high_info);
    let mut old_salt = Sha256::new();
    old_salt.update(b"mewtual/voice/v2/transcript\0");
    old_salt.update(t.0);
    assert_ne!(t.salt(), <[u8; 32]>::from(old_salt.finalize()));
}

#[test]
fn point_validation_is_explicitly_shape_and_reference_only_not_curve_or_ownership_proof() {
    let (mut a, mut low, high) = keys();
    low.point = [0; 65];
    low.point[0] = 4; // Not an on-curve claim.
    a.low.key_ref = key_reference(&low.point);
    assert!(Transcript::prepare(&a, &low, &high).is_ok());
    low.point[0] = 2;
    assert!(Transcript::prepare(&a, &low, &high).is_err());
    low.point[0] = 4;
    low.point[64] = 1;
    assert!(Transcript::prepare(&a, &low, &high).is_err());
}
