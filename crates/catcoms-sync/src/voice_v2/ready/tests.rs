use super::*;

#[test]
fn unpaid_ready_peek_has_exact_native_fields_hash_and_fixed_helpers_with_no_trailing_admission() {
    let (_, high, scope, mut header) = fixture();
    header.helpers = vec![
        DeviceKeypair::from_seed(&[3; 32]).device_id(),
        DeviceKeypair::from_seed(&[4; 32]).device_id(),
    ];
    header.helpers.sort();
    let object = SignedObject::issue(
        scope,
        &high,
        [18; 32],
        1000,
        16000,
        Body::PairReadyRound {
            sequence: 1,
            header: header.clone(),
            profile: SdpProfile::CompleteSdpV1,
        },
    )
    .unwrap();
    let direct = Envelope::Direct(object.clone());
    let peek = Envelope::peek_ready(&direct.encode()).unwrap();
    assert_eq!(
        (peek.issuer, peek.message, peek.hash, peek.scope),
        (object.issuer(), object.id(), object.hash(), scope)
    );
    assert_eq!(peek.header.epoch, header.epoch);
    assert_eq!(peek.header.finish_message, header.finish_message);
    assert_eq!(peek.header.finish_object_hash, header.finish_object_hash);
    assert_eq!(
        peek.header.helpers,
        [Some(header.helpers[0]), Some(header.helpers[1])]
    );
    for helper in &header.helpers {
        let forwarded = Envelope::HelperDelivery {
            object: object.clone(),
            permit: RoutePermit::issue(&high, &object, *helper, 1000, 16000).unwrap(),
        };
        let peek = Envelope::peek_ready(&forwarded.encode()).unwrap();
        assert_eq!(peek.envelope_kind, 3);
        assert_eq!(peek.hash, object.hash());
    }
    let mut trailing = direct.encode();
    trailing.push(0);
    assert!(Envelope::peek_ready(&trailing).is_err());
    // Change the declared count without constructing any Vec in the unpaid parser.
    let mut unsigned = object.unsigned();
    let count = unsigned.len() - 2 * (4 + 32) - 10;
    assert_eq!(unsigned[count], 2);
    for value in [3, 255] {
        unsigned[count] = value;
        let mut signed = Encoder::new();
        fixed(&mut signed, &unsigned);
        fixed(&mut signed, &object.signature);
        let mut outer = Encoder::new();
        outer.put_u8(VERSION);
        outer.put_u8(1);
        fixed(&mut outer, &signed.finish());
        assert!(matches!(
            Envelope::peek_ready(&outer.finish()),
            Err(VoiceError::Bounds)
        ));
    }
}

fn fixture() -> (DeviceKeypair, DeviceKeypair, Scope, ReadyHeader) {
    let mut devices = [
        DeviceKeypair::from_seed(&[1; 32]),
        DeviceKeypair::from_seed(&[2; 32]),
    ];
    devices.sort_by_key(DeviceKeypair::device_id);
    let [low, high] = devices;
    let scope = Scope::for_group(b"ready wire boundary", Some([8; 32]), 99);
    let header = ReadyHeader {
        epoch: 7,
        target: low.device_id(),
        source_join: [11; 32],
        target_join: [12; 32],
        source_key_ref: [13; 32],
        target_key_ref: [14; 32],
        handshake: [15; 32],
        finish_message: [16; 32],
        finish_object_hash: [17; 32],
        helpers: vec![],
    };
    (low, high, scope, header)
}
fn issue(
    high: &DeviceKeypair,
    scope: Scope,
    header: ReadyHeader,
) -> Result<SignedObject, VoiceError> {
    SignedObject::issue(
        scope,
        high,
        [18; 32],
        1000,
        16_000,
        Body::PairReady { header },
    )
}

#[test]
fn maximum_two_helper_ready_roundtrips_with_the_same_immutable_canonical_list() {
    let (_, high, scope, mut header) = fixture();
    header.helpers = vec![
        DeviceKeypair::from_seed(&[3; 32]).device_id(),
        DeviceKeypair::from_seed(&[4; 32]).device_id(),
    ];
    header.helpers.sort();
    let object = issue(&high, scope, header.clone()).unwrap();
    let decoded = SignedObject::decode(&object.encode()).unwrap();
    assert_eq!(decoded.body(), &Body::PairReady { header });
    assert_eq!(decoded.encode(), object.encode());
}
fn context(scope: Scope, local: DeviceId) -> Context {
    Context {
        scope,
        local,
        now: 2000,
        epoch: 7,
        indirect: true,
    }
}

#[test]
fn ready_roundtrips_as_closed_tag_five_and_never_exposes_encrypted_aad() {
    let (low, high, scope, header) = fixture();
    let object = issue(&high, scope, header).unwrap();
    assert_eq!(object.body.kind(), 5);
    assert_eq!(object.body.target(), Some(low.device_id()));
    assert_eq!(SignedObject::decode(&object.encode()).unwrap(), object);
    let envelope = Envelope::Direct(object.clone());
    assert_eq!(
        Envelope::decode(&envelope.encode()).unwrap().object(),
        &object
    );
    assert_eq!(
        envelope.authorize(high.device_id(), &context(scope, low.device_id()), |_| true),
        Ok(Disposition::Deliver)
    );
    assert_eq!(object.aad(), Err(VoiceError::Malformed));
    assert_eq!(format!("{:?}", object.body), "Body { .. }");
    // Neither a signed wire object nor a live join can manufacture the private future Ready
    // Source owner. Until that factory exists, ordinary joined activity refuses this body.
    let key = JoinKey::new([11; 32], super::super::tests::key(1).public_key).unwrap();
    let activity =
        source::SourceActivity::joined(scope, high.device_id(), key, CallLease::default());
    assert_eq!(activity.check(&object), Err(VoiceError::Unauthorized));
}

#[test]
fn ready_rejects_wrong_direction_and_noncanonical_or_endpoint_helpers_before_signing() {
    let (low, high, scope, header) = fixture();
    let mut reversed = header.clone();
    reversed.target = high.device_id();
    assert_eq!(issue(&low, scope, reversed), Err(VoiceError::Malformed));
    let a = DeviceKeypair::from_seed(&[3; 32]).device_id();
    let b = DeviceKeypair::from_seed(&[4; 32]).device_id();
    let mut helpers = [a, b];
    helpers.sort();
    for invalid in [
        vec![helpers[1], helpers[0]],
        vec![a, a],
        vec![low.device_id()],
        vec![high.device_id()],
    ] {
        let mut changed = header.clone();
        changed.helpers = invalid;
        assert_eq!(issue(&high, scope, changed), Err(VoiceError::Malformed));
    }
    let mut changed = header.clone();
    changed.helpers = vec![a, b, a];
    assert_eq!(issue(&high, scope, changed), Err(VoiceError::Bounds));
    let mut direct = scope;
    direct.policy = None;
    let mut changed = header;
    changed.helpers = vec![a];
    assert_eq!(issue(&high, direct, changed), Err(VoiceError::Unauthorized));
}

#[test]
fn exact_epoch_helpers_and_permitted_helper_route_are_independent_checks() {
    let (low, high, scope, mut header) = fixture();
    let helper = DeviceKeypair::from_seed(&[3; 32]).device_id();
    let unrelated = DeviceKeypair::from_seed(&[4; 32]).device_id();
    header.helpers = vec![helper];
    let object = issue(&high, scope, header).unwrap();
    let permit = RoutePermit::issue(&high, &object, helper, 1000, 16_000).unwrap();
    let envelope = Envelope::HelperDelivery {
        object: object.clone(),
        permit,
    };
    let expected = context(scope, low.device_id());
    assert_eq!(
        envelope.authorize(helper, &expected, |_| true),
        Ok(Disposition::Deliver)
    );
    for epoch in [6, 8, u64::MAX] {
        let changed = Context { epoch, ..expected };
        assert_eq!(
            envelope.authorize(helper, &changed, |_| true),
            Err(VoiceError::Unauthorized)
        );
    }
    let direct_only = Context {
        indirect: false,
        ..expected
    };
    assert_eq!(
        Envelope::Direct(object.clone()).authorize(high.device_id(), &direct_only, |_| true),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        Envelope::Direct(object.clone())
            .authorize(high.device_id(), &expected, |member| member != helper),
        Err(VoiceError::Unauthorized)
    );
    let changed = Envelope::HelperDelivery {
        permit: RoutePermit::issue(&high, &object, unrelated, 1000, 16_000).unwrap(),
        object,
    };
    assert_eq!(
        changed.authorize(unrelated, &expected, |_| true),
        Err(VoiceError::Unauthorized)
    );
}

#[test]
fn every_ready_field_and_helper_is_signed_and_decoder_bounds_count_before_allocation() {
    let (_, high, scope, mut header) = fixture();
    header.helpers = vec![DeviceKeypair::from_seed(&[3; 32]).device_id()];
    let object = issue(&high, scope, header).unwrap();
    let encoded = object.encode();
    for offset in 0..encoded.len() {
        let mut changed = encoded.clone();
        changed[offset] ^= 1;
        assert!(
            SignedObject::decode(&changed).is_err(),
            "accepted altered Ready byte{offset}"
        );
    }
    let mut trailing = encoded;
    trailing.push(0);
    assert!(SignedObject::decode(&trailing).is_err());
    let (_, _, _, header) = fixture();
    let mut encoding = Encoder::new();
    header.put(&mut encoding);
    let mut bytes = encoding.finish();
    *bytes.last_mut().unwrap() = 3; // count, with NO allocation/read of helper bytes allowed
    assert_eq!(
        ReadyHeader::get(&mut Decoder::new(&bytes)),
        Err(VoiceError::Bounds)
    );
    // The existing signing domains/version and tags remain unchanged.
    assert_eq!(OBJECT_DOMAIN, "mewtual/voice/object/v2");
    assert_eq!(VERSION, 2);
    assert_eq!(super::super::tests::announcement(&high, 1).body.kind(), 1);
}

#[test]
fn new_ready_profile_is_closed_and_preserves_legacy_header_bytes_and_frozen_helper_route() {
    let (low, high, scope, mut header) = fixture();
    let helper = DeviceKeypair::from_seed(&[3; 32]).device_id();
    let other = DeviceKeypair::from_seed(&[4; 32]).device_id();
    header.helpers = vec![helper];
    let old = issue(&high, scope, header.clone()).unwrap();
    let new = SignedObject::issue(
        scope,
        &high,
        old.id(),
        old.issued(),
        old.expires(),
        Body::PairReadyRound {
            sequence: 1,
            header: header.clone(),
            profile: SdpProfile::CompleteSdpV1,
        },
    )
    .unwrap();
    assert_eq!(new.body.kind(), 6);
    assert_eq!(new.encode().len(), old.encode().len() + 9);
    assert_eq!(SignedObject::decode(&new.encode()).unwrap(), new);
    // Tag5 still decodes but cannot select the protected native readiness verifier.
    assert!(Envelope::peek_ready(&Envelope::Direct(old.clone()).encode()).is_err());
    let mut legacy = Encoder::new();
    old.body.put_header(&mut legacy);
    let mut round = Encoder::new();
    new.body.put_header(&mut round);
    let mut expected = legacy.finish();
    expected.extend_from_slice(&1u64.to_be_bytes());
    expected.push(1);
    assert_eq!(round.finish(), expected);
    // Every byte, including mode, remains signed. Unknown profile fails parsing, rather
    // than silently interpreting this body as the older tag5 form.
    for mode in [0, 2, 255] {
        let mut unsigned = new.unsigned();
        *unsigned.last_mut().unwrap() = mode;
        let mut encoded = Encoder::new();
        fixed(&mut encoded, &unsigned);
        fixed(&mut encoded, &new.signature);
        assert!(matches!(
            SignedObject::decode(&encoded.finish()),
            Err(VoiceError::Malformed)
        ));
    }
    let context = context(scope, low.device_id());
    for (candidate, admitted) in [(helper, true), (other, false)] {
        let forwarded = Envelope::HelperDelivery {
            object: new.clone(),
            permit: RoutePermit::issue(&high, &new, candidate, 1000, 16000).unwrap(),
        };
        assert_eq!(
            forwarded.authorize(candidate, &context, |_| true).is_ok(),
            admitted
        );
    }
    let mut unsigned = new.unsigned();
    unsigned.push(0);
    let mut encoded = Encoder::new();
    fixed(&mut encoded, &unsigned);
    fixed(&mut encoded, &new.signature);
    assert!(SignedObject::decode(&encoded.finish()).is_err());
}

#[test]
fn ready_sequence_has_full_u64_closed_encoding_and_zero_never_signs() {
    let (_, high, scope, header) = fixture();
    assert_eq!(
        SignedObject::issue(
            scope,
            &high,
            [81; 32],
            1000,
            16000,
            Body::PairReadyRound {
                header: header.clone(),
                sequence: 0,
                profile: SdpProfile::CompleteSdpV1
            }
        ),
        Err(VoiceError::Malformed)
    );
    for sequence in [1, 2, u64::MAX] {
        let body = Body::PairReadyRound {
            header: header.clone(),
            sequence,
            profile: SdpProfile::CompleteSdpV1,
        };
        let object = SignedObject::issue(scope, &high, [81; 32], 1000, 16000, body).unwrap();
        assert_eq!(SignedObject::decode(&object.encode()).unwrap(), object);
        assert_eq!(
            Envelope::peek_ready(&Envelope::Direct(object.clone()).encode())
                .unwrap()
                .sequence,
            sequence
        );
        let unsigned = object.unsigned();
        assert_eq!(
            &unsigned[unsigned.len() - 9..unsigned.len() - 1],
            &sequence.to_be_bytes()
        );
    }
}
