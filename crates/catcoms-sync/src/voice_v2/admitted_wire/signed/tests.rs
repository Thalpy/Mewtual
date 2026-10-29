use super::*;
use crate::voice_v2::admitted_wire::fixtures as f;

fn encrypted(scope: Scope, kind: u8, profile: u8, plaintext: usize) -> (Vec<u8>, usize) {
    let (low, high) = f::identities();
    let from_low = matches!(kind, 1 | 3 | 7 | 8);
    let (source, target) = if from_low {
        (&low, &high)
    } else {
        (&high, &low)
    };
    let mut e = Encoder::with_capacity(340 + plaintext + 20);
    e.put_u8(ADMISSION_PROFILE);
    fixed(&mut e, &[1; 32]);
    fixed(&mut e, target.device_id().as_bytes());
    fixed(&mut e, &[2; 32]);
    fixed(&mut e, &[3; 32]);
    fixed(&mut e, &[4; 32]);
    e.put_u8(SUITE);
    fixed(&mut e, &[5; 32]);
    e.put_u8(u8::from(from_low));
    e.put_u8(kind);
    e.put_u64(u64::MAX);
    e.put_u8(u8::from(matches!(kind, 8 | 9)));
    if matches!(kind, 8 | 9) {
        fixed(&mut e, low.device_id().as_bytes());
        fixed(&mut e, &[6; 32]);
        e.put_u8(profile);
        fixed(&mut e, &[7; 32]);
    }
    let header_len = e.len();
    fixed(&mut e, &vec![8; plaintext + 16]);
    (
        f::signed(8, source, scope, [6; 32], 1000, 16000, &e.finish()),
        header_len,
    )
}
fn ready(scope: Scope, helpers: &[DeviceId], profile: u8, sequence: u64) -> Vec<u8> {
    let (low, high) = f::identities();
    let mut e = Encoder::new();
    e.put_u8(ADMISSION_PROFILE);
    e.put_u64(7);
    fixed(&mut e, low.device_id().as_bytes());
    for value in 1..=7 {
        fixed(&mut e, &[value; 32]);
    }
    e.put_u8(helpers.len() as u8);
    for helper in helpers {
        fixed(&mut e, helper.as_bytes());
    }
    e.put_u64(sequence);
    e.put_u8(profile);
    f::signed(9, &high, scope, [9; 32], 1000, 16000, &e.finish())
}

/// Borrow the test object's body without normalizing it through either decoder.
/// This lets alias tests retain a valid signature around deliberately old tags.
fn body_bytes(bytes: &[u8]) -> &[u8] {
    let mut outer = Decoder::new(bytes);
    let unsigned = outer.get_bytes().unwrap();
    let mut d = Decoder::new(unsigned);
    d.get_str().unwrap();
    d.get_u8().unwrap();
    d.get_u8().unwrap();
    Scope::get(&mut d).unwrap();
    for _ in 0..3 {
        let _: [u8; 32] = array(&mut d).unwrap();
    }
    d.get_u64().unwrap();
    d.get_u64().unwrap();
    &unsigned[unsigned.len() - d.remaining()..]
}

fn assert_old_direct_ingress_closed(object: &[u8]) {
    let mut e = Encoder::new();
    e.put_u8(VERSION);
    e.put_u8(1);
    fixed(&mut e, object);
    let frame = e.finish();
    assert!(Envelope::decode(&frame).is_err());
    assert!(Envelope::peek_ready(&frame).is_err());
    assert!(Envelope::peek_encrypted(&frame).is_err());
}

#[test]
fn proposed_control_envelope_widths_are_executed_but_old_ingress_stays_closed() {
    let (low, high) = f::identities();
    for policy in [false, true] {
        let scope = f::scope(policy);
        let request = f::request(scope, &f::availability(scope, true));
        let request_view = SignedAdmitted::decode(&request).unwrap();
        for kind in 1..=3 {
            if !policy && kind != 1 {
                continue;
            } // Absent policy remains direct-only.
            let mut e = Encoder::new();
            e.put_u8(VERSION);
            e.put_u8(kind);
            fixed(&mut e, &request);
            if kind != 1 {
                // Structural wire-width evidence only: this test-local framing
                // uses the unchanged permit grammar. It is not a production
                // new-family permit issuer or forwarding validator.
                let mut permit = RoutePermit {
                    scope,
                    issuer: low.device_id(),
                    public_key: low.verifying_key().to_bytes(),
                    packet: request_view.hash(),
                    helper: DeviceKeypair::from_seed(&[3; 32]).device_id(),
                    final_scope: FinalScope::Endpoint(high.device_id()),
                    issued: 2000,
                    expires: 17000,
                    signature: [0; 64],
                };
                permit.signature = low.sign(&permit.unsigned());
                let permit = permit.encode();
                assert_eq!(permit.len(), 392);
                assert!(RoutePermit::decode(&permit).is_ok());
                fixed(&mut e, &permit);
            }
            let frame = e.finish();
            assert_eq!(
                frame.len(),
                if kind != 1 {
                    1544
                } else if policy {
                    1148
                } else {
                    1076
                }
            );
            assert!(frame.len() < MAX_ENVELOPE);
            assert!(Envelope::decode(&frame).is_err());
            assert!(Envelope::peek_ready(&frame).is_err());
            assert!(Envelope::peek_encrypted(&frame).is_err());
        }
    }
}

#[test]
fn old_tags_cannot_accept_admitted_profile_by_removing_only_the_family_byte() {
    let (low, high) = f::identities();
    let scope = f::scope(true);
    for (kind, signer, id, body) in [
        (4, &low, [6; 32], encrypted(scope, 8, 2, 0).0),
        (6, &high, [9; 32], ready(scope, &[], 2, 1)),
    ] {
        let body = body_bytes(&body);
        assert_eq!(body[0], ADMISSION_PROFILE);
        let alias = f::signed(kind, signer, scope, id, 1000, 16000, &body[1..]);
        assert!(
            SignedObject::decode(&alias).is_err(),
            "old kind {kind} refuses profile2"
        );
        assert!(
            SignedAdmitted::decode(&alias).is_err(),
            "new decoder refuses old kind {kind}"
        );
    }
}

#[test]
fn freshly_resigned_encrypted_direction_sequence_and_offer_identity_must_match() {
    let (low, _) = f::identities();
    let scope = f::scope(true);
    let original = encrypted(scope, 8, 2, 0).0;
    let body = body_bytes(&original);
    for offset in [218, 233, 269] {
        let mut bad = body.to_vec();
        bad[offset] ^= 1;
        let signed = f::signed(8, &low, scope, [6; 32], 1000, 16000, &bad);
        assert!(
            SignedAdmitted::decode(&signed).is_err(),
            "encrypted binding byte {offset}"
        );
    }
    let mut zero_sequence = body.to_vec();
    zero_sequence[220..228].fill(0);
    assert!(SignedAdmitted::decode(&f::signed(
        8,
        &low,
        scope,
        [6; 32],
        1000,
        16000,
        &zero_sequence
    ))
    .is_err());
}

#[test]
fn family_specific_control_and_ready_ceilings_precede_payload_decoding() {
    let (_, high) = f::identities();
    let scope = f::scope(true);
    // A syntactically invalid payload still demonstrates which exact family
    // ceiling applies, without mistaking extra signed bytes for a valid control.
    for extra in [0, 1] {
        let payload = vec![0; MAX_CONTROL - 318 + extra];
        let bytes = f::signed(7, &high, scope, [10; 32], 1000, 16000, &payload);
        assert_eq!(bytes.len(), MAX_CONTROL + extra);
        if extra == 0 {
            assert!(matches!(
                SignedAdmitted::decode(&bytes),
                Err(VoiceError::Malformed)
            ));
        } else {
            assert!(matches!(
                SignedAdmitted::decode(&bytes),
                Err(VoiceError::Bounds)
            ));
        }
    }
    let helpers = [DeviceId::from_bytes([1; 32]), DeviceId::from_bytes([2; 32])];
    let bytes = ready(scope, &helpers, 2, 1);
    assert_eq!(bytes.len(), MAX_READY);
    assert!(SignedAdmitted::decode(&bytes).is_ok());
    let mut oversized = body_bytes(&bytes).to_vec();
    oversized.push(0);
    let oversized = f::signed(9, &high, scope, [9; 32], 1000, 16000, &oversized);
    assert_eq!(oversized.len(), MAX_READY + 1);
    assert!(matches!(
        SignedAdmitted::decode(&oversized),
        Err(VoiceError::Bounds)
    ));
}

#[test]
fn every_control_form_has_exact_policy_vectors_and_separate_old_decoder_rejection() {
    let (low, high) = f::identities();
    for policy in [false, true] {
        let scope = f::scope(policy);
        let overhead = if policy { 318 } else { 282 };
        let target = low.device_id();
        let mut cases = Vec::new();
        for kind in [0, 1] {
            let mut tail = target.as_bytes().to_vec();
            tail.push(kind);
            cases.push((f::payload(0, 7, &tail), 44));
        }
        let mut support = target.as_bytes().to_vec();
        support.extend([21; 32]);
        support.extend(7u16.to_be_bytes());
        cases.push((f::payload(1, 7, &support), 77));
        let mut busy = target.as_bytes().to_vec();
        busy.extend([21; 32]);
        busy.extend(1000u16.to_be_bytes());
        cases.push((f::payload(6, 7, &busy), 77));
        let mut status = target.as_bytes().to_vec();
        status.extend([21; 32]);
        cases.push((f::payload(7, 7, &status), 75));
        cases.push((f::terminal_payload(scope, true, 6), 184));
        cases.push((f::terminal_payload(scope, false, 2), 184));
        for (payload, width) in cases {
            assert_eq!(payload.len(), width);
            let bytes = f::signed(7, &high, scope, [31; 32], 1000, 16000, &payload);
            assert_eq!(bytes.len(), overhead + width);
            assert_eq!(SignedAdmitted::decode(&bytes).unwrap().encode(), bytes);
            assert!(SignedObject::decode(&bytes).is_err());
        }
        let cap = f::availability(scope, true);
        let request = f::request(scope, &cap);
        let request_view = SignedAdmitted::decode(&request).unwrap();
        assert_eq!(request.len(), overhead + 317 + cap.len());
        let grant = f::grant(scope, request_view.hash(), 15000);
        assert_eq!(grant.len(), overhead + 117);
        assert!(SignedAdmitted::decode(&grant).is_ok());
        let mut commit = high.device_id().as_bytes().to_vec();
        commit.extend([0; 160]);
        let bytes = f::signed(
            7,
            &low,
            scope,
            [32; 32],
            1000,
            16000,
            &f::payload(5, 7, &commit),
        );
        assert_eq!(bytes.len(), overhead + 203);
        assert!(SignedAdmitted::decode(&bytes).is_ok());
    }
}

#[test]
fn resigned_historical_terminal_keeps_immutable_digest_and_tolerates_wrapper_clock_rollback() {
    let (_, high) = f::identities();
    let scope = f::scope(true);
    let original = f::terminal_payload(scope, false, 1);
    let first_bytes = f::signed(7, &high, scope, [51; 32], 1000, 16000, &original);
    let first = SignedAdmitted::decode(&first_bytes).unwrap();
    for observed in 0..=5 {
        let payload = f::terminal_payload(scope, false, observed);
        assert_eq!(&payload[151..183], &original[151..183]);
        let bytes = f::signed(7, &high, scope, [52; 32], 0, 1, &payload);
        let value = SignedAdmitted::decode(&bytes).unwrap();
        assert_ne!(
            value.hash(),
            first.hash(),
            "wrapper hash differs, immutable result does not"
        );
    }
    let mut wrong = original;
    wrong[151] ^= 1;
    assert!(
        SignedAdmitted::decode(&f::signed(7, &high, scope, [53; 32], 1000, 16000, &wrong)).is_err()
    );
}

#[test]
fn new_family_minimum_maximum_vectors_preserve_exact_bounds_and_borrow_original_input() {
    for policy in [false, true] {
        let scope = f::scope(policy);
        let cap = f::availability(scope, true);
        let cap_decoded = SignedAdmitted::decode(&cap).unwrap();
        assert_eq!(cap_decoded.encode(), cap);
        assert_eq!(cap_decoded.canonical.as_ptr(), cap.as_ptr());
        assert!(SignedObject::decode(&cap).is_err());
        assert_old_direct_ingress_closed(&cap);
        let (bytes, body_header) = encrypted(scope, 8, 2, MAX_SDP_PLAINTEXT);
        let value = SignedAdmitted::decode(&bytes).unwrap();
        assert_eq!(bytes.len(), if policy { 8864 } else { 8828 });
        assert_eq!(value.aad().unwrap().len(), if policy { 580 } else { 544 });
        assert_eq!(
            value.aad().unwrap().len(),
            if policy { 242 } else { 206 } + body_header
        );
        assert_eq!(value.encode(), bytes);
        assert!(SignedObject::decode(&bytes).is_err());
        assert_old_direct_ingress_closed(&bytes);
        let AdmittedBody::Encrypted(body) = &value.body else {
            panic!("encrypted view");
        };
        let start = bytes.as_ptr() as usize;
        let end = start + bytes.len();
        assert!((start..end).contains(&(body.ciphertext.as_ptr() as usize)));
        assert_eq!(body.ciphertext.len(), MAX_SDP_PLAINTEXT + 16);
        let helpers = if policy {
            vec![DeviceId::from_bytes([1; 32]), DeviceId::from_bytes([2; 32])]
        } else {
            vec![]
        };
        let bytes = ready(scope, &helpers, 2, u64::MAX);
        assert_eq!(bytes.len(), if policy { 693 } else { 585 });
        assert_eq!(SignedAdmitted::decode(&bytes).unwrap().encode(), bytes);
        assert!(SignedObject::decode(&bytes).is_err());
        assert_old_direct_ingress_closed(&bytes);
    }
}

#[test]
fn admitted_encrypted_kinds_preserve_causality_and_reject_old_sdp_ice_and_profile_aliases() {
    for kind in [1, 2, 3, 7, 8, 9] {
        let maximum = match kind {
            8 | 9 => MAX_SDP_PLAINTEXT,
            7 => 128,
            _ => 512,
        };
        let (valid, _) = encrypted(f::scope(true), kind, 2, maximum);
        assert!(SignedAdmitted::decode(&valid).is_ok(), "kind {kind}");
        let (too_big, _) = encrypted(f::scope(true), kind, 2, maximum + 1);
        assert!(SignedAdmitted::decode(&too_big).is_err());
    }
    for kind in [0, 4, 5, 6, 10, 255] {
        assert!(SignedAdmitted::decode(&encrypted(f::scope(true), kind, 2, 0).0).is_err());
    }
    for profile in [0, 1, 3, 255] {
        assert!(SignedAdmitted::decode(&encrypted(f::scope(true), 8, profile, 0).0).is_err());
        assert!(SignedAdmitted::decode(&ready(f::scope(true), &[], profile, 1)).is_err());
    }
    assert!(SignedAdmitted::decode(&ready(f::scope(true), &[], 2, 0)).is_err());
}

#[test]
fn fixed_ready_helpers_retain_high_role_policy_sorted_distinct_and_endpoint_exclusion() {
    let (low, high) = f::identities();
    let first = DeviceId::from_bytes([1; 32]);
    let second = DeviceId::from_bytes([2; 32]);
    for helpers in [
        vec![first, first],
        vec![second, first],
        vec![low.device_id()],
        vec![high.device_id()],
        vec![first, second, DeviceId::from_bytes([3; 32])],
    ] {
        assert!(SignedAdmitted::decode(&ready(f::scope(true), &helpers, 2, 1)).is_err());
    }
    assert!(SignedAdmitted::decode(&ready(f::scope(false), &[first], 2, 1)).is_err());
}

#[test]
fn old_signed_objects_remain_separate_and_new_family_is_strict_on_signatures_and_framing() {
    let (low, _) = f::identities();
    let mut point = [0; 65];
    point[0] = 4;
    let old = SignedObject::issue(
        f::scope(true),
        &low,
        [0; 32],
        1000,
        16000,
        Body::Announcement {
            key: JoinKey::new([0; 32], point).unwrap(),
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap();
    let old_bytes = old.encode();
    assert_eq!(SignedObject::decode(&old_bytes).unwrap(), old);
    assert!(SignedAdmitted::decode(&old_bytes).is_err());
    let bytes = f::availability(f::scope(true), true);
    for end in 0..bytes.len() {
        assert!(SignedAdmitted::decode(&bytes[..end]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(SignedAdmitted::decode(&trailing).is_err());
    let mut bad_signature = bytes.clone();
    let last = bad_signature.len() - 1;
    bad_signature[last] ^= 1;
    assert!(matches!(
        SignedAdmitted::decode(&bad_signature),
        Err(VoiceError::Signature)
    ));
    assert!(SignedAdmitted::decode(&vec![0; MAX_ENCRYPTED + 1]).is_err());
    assert!(SignedAdmitted::decode_availability(&encrypted(f::scope(true), 1, 2, 0).0).is_err());
}

#[test]
fn codec_layout_observations_are_not_native_payer_or_activation_proof() {
    eprintln!(
        "admitted codec inline signed {}, body {}, control {}, header {}; maxima2048/8864/693",
        std::mem::size_of::<SignedAdmitted<'_>>(),
        std::mem::size_of::<AdmittedBody<'_>>(),
        std::mem::size_of::<Control<'_>>(),
        std::mem::size_of::<Header>()
    );
    eprintln!("admitted codec inline request {}, encrypted {}, ready {}, agreement {}, endpoint {}, recorded {}",
        std::mem::size_of::<control::Request<'_>>(), std::mem::size_of::<Encrypted<'_>>(),
        std::mem::size_of::<Ready>(), std::mem::size_of::<agreement::Agreement>(),
        std::mem::size_of::<agreement::Endpoint>(), std::mem::size_of::<control::Recorded>());
    assert_eq!(MAX_ENVELOPE, 48 * 1024);
    // These fixed wire ceilings do NOT assert that a future native task/Sourcebook
    // may capture this value without its own complete prepaid compiled inventory.
    assert_eq!((MAX_CONTROL, MAX_ENCRYPTED, MAX_READY), (2048, 8864, 693));
}
