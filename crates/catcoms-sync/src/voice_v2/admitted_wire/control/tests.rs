use super::*;
use crate::voice_v2::admitted_wire::fixtures as f;

fn common() -> Header {
    let (_, high) = f::identities();
    Header {
        scope: f::scope(true),
        issuer: high.device_id(),
        public_key: high.verifying_key().to_bytes(),
        id: [99; 32],
        issued: 3000,
        expires: 18000,
    }
}
fn target() -> DeviceId {
    f::identities().0.device_id()
}

struct TerminalFields {
    decision: u8,
    reason: u8,
    grant: bool,
    round: bool,
    at_wall: u64,
    observed: u8,
}
fn terminal(header: &Header, epoch: u64, t: TerminalFields) -> Vec<u8> {
    // Independently framed immutable record vector, not Recorded::digest or RawOutput.
    let mut record = vec![1, 1];
    record.extend(header.scope.group);
    record.push(u8::from(header.scope.policy.is_some()));
    record.extend(header.scope.policy.unwrap_or([0; 32]));
    record.extend(header.scope.room.to_be_bytes());
    record.extend(epoch.to_be_bytes());
    record.extend(header.issuer.as_bytes());
    record.extend(target().as_bytes());
    record.extend([55; 32]);
    record.push(u8::from(t.grant));
    record.extend([0; 32]);
    record.push(u8::from(t.round));
    record.extend([0; 32]);
    record.extend([t.decision, t.reason]);
    record.extend(t.at_wall.to_be_bytes());
    assert_eq!(record.len(), 263);
    let mut preimage = b"mewtual/voice/admission/terminal/v1\0".to_vec();
    preimage.extend(&record);
    assert_eq!(preimage.len(), 299);
    let digest = if t.decision == 0 {
        [0; 32]
    } else {
        *blake3::hash(&preimage).as_bytes()
    };
    let mut tail = Vec::new();
    tail.extend(target().as_bytes());
    tail.extend([55; 32]);
    tail.push(u8::from(t.grant));
    tail.extend([0; 32]);
    tail.push(u8::from(t.round));
    tail.extend([0; 32]);
    tail.extend([t.decision, t.reason]);
    tail.extend(t.at_wall.to_be_bytes());
    tail.extend(digest);
    tail.push(t.observed);
    f::payload(8, epoch, &tail)
}

#[test]
fn closed_query_features_availability_busy_and_all_scalar_boundaries() {
    let h = common();
    for kind in 0..=2 {
        let mut tail = target().as_bytes().to_vec();
        tail.push(kind);
        let payload = f::payload(0, 7, &tail);
        assert_eq!(Control::decode(&payload, &h).is_ok(), kind < 2);
    }
    for features in [0u16, 1, 2, 4, 7, 8, u16::MAX] {
        let mut tail = target().as_bytes().to_vec();
        tail.extend([1; 32]);
        tail.extend(features.to_be_bytes());
        let payload = f::payload(1, 7, &tail);
        assert_eq!(Control::decode(&payload, &h).is_ok(), features & !7 == 0);
    }
    for hint in [0u16, 1, 1000, 1001, u16::MAX] {
        let mut tail = target().as_bytes().to_vec();
        tail.extend([1; 32]);
        tail.extend(hint.to_be_bytes());
        let payload = f::payload(6, 7, &tail);
        assert_eq!(Control::decode(&payload, &h).is_ok(), hint <= 1000);
    }
    for available in 0..=2u8 {
        for layout in 0..=2u8 {
            let mut tail = target().as_bytes().to_vec();
            tail.extend([0; 128]);
            tail.extend(u64::MAX.to_be_bytes());
            tail.extend(u64::MAX.to_be_bytes());
            tail.extend([available, layout]);
            let payload = f::payload(2, 7, &tail);
            assert_eq!(
                Control::decode(&payload, &h).is_ok(),
                available < 2 && available == layout
            );
        }
    }
}

#[test]
fn positive_nested_capability_and_exact_tuple_scope_epoch_signature_are_required() {
    for policy in [false, true] {
        let scope = f::scope(policy);
        let cap = f::availability(scope, true);
        let request = f::request(scope, &cap);
        assert_eq!(cap.len(), if policy { 507 } else { 471 });
        assert_eq!(request.len(), if policy { 1142 } else { 1070 });
        let view = SignedAdmitted::decode(&request).unwrap();
        let AdmittedBody::Control(Control {
            operation: Operation::Request(value),
            ..
        }) = view.body()
        else {
            panic!("request view");
        };
        assert_eq!(value.capability, cap);
        let original = request.as_ptr() as usize;
        assert!(
            (original..original + request.len()).contains(&(value.capability.as_ptr() as usize)),
            "the retained capability borrows the same original signed Request"
        );
        let negative = f::availability(scope, false);
        assert!(SignedAdmitted::decode(&f::request(scope, &negative)).is_err());
        let foreign = f::availability(f::scope(!policy), true);
        assert!(SignedAdmitted::decode(&f::request(scope, &foreign)).is_err());
        let mut damaged = cap.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(SignedAdmitted::decode(&f::request(scope, &damaged)).is_err());
        assert!(SignedAdmitted::decode(&f::request(scope, &vec![0; 513])).is_err());
        // A signed Request cannot be recursively promoted into a capability.
        assert!(SignedAdmitted::decode(&f::request(scope, &request)).is_err());
    }
}

#[test]
fn freshly_resigned_capability_mismatches_pin_each_exact_nested_binding() {
    let scope = f::scope(true);
    let (_, high) = f::identities();
    let original = f::availability(scope, true);
    let original_payload = f::control_payload(&original);
    // Epoch, target LOW, HIGH join/key/incarnation/generation each differs ALONE.
    // All resulting capabilities retain valid outer signatures and syntax.
    for offset in [3, 11, 75, 107, 139, 171] {
        let mut payload = original_payload.clone();
        payload[offset] ^= 1;
        let capability = f::signed(7, &high, scope, [22; 32], 1000, 16000, &payload);
        assert!(
            SignedAdmitted::decode(&capability).is_ok(),
            "signed capability field {offset}"
        );
        let request = f::request(scope, &capability);
        assert!(
            SignedAdmitted::decode(&request).is_err(),
            "nested exact field {offset}"
        );
    }
    let foreign_issuer = DeviceKeypair::from_seed(&[3; 32]);
    let capability = f::signed(
        7,
        &foreign_issuer,
        scope,
        [22; 32],
        1000,
        16000,
        &original_payload,
    );
    assert!(SignedAdmitted::decode(&capability).is_ok());
    assert!(SignedAdmitted::decode(&f::request(scope, &capability)).is_err());
    // Availability's own nonzero generation/serial boundaries are independent
    // of Request tuple equality and accept MAX without inventing advancement.
    for start in [171, 179] {
        let mut payload = original_payload.clone();
        payload[start..start + 8].fill(0);
        let capability = f::signed(7, &high, scope, [22; 32], 1000, 16000, &payload);
        assert!(SignedAdmitted::decode(&capability).is_err());
    }
}

#[test]
fn freshly_resigned_request_role_self_issuer_and_zero_generation_boundaries_refuse() {
    let scope = f::scope(true);
    let (low, high) = f::identities();
    let cap = f::availability(scope, true);
    let original = f::request(scope, &cap);
    let payload = f::control_payload(&original);
    assert!(
        SignedAdmitted::decode(&f::signed(7, &high, scope, [23; 32], 2000, 17000, &payload))
            .is_err()
    );
    let unrelated = DeviceKeypair::from_seed(&[3; 32]);
    assert!(
        SignedAdmitted::decode(&f::signed(
            7, &unrelated, scope, [23; 32], 2000, 17000, &payload
        ))
        .is_err(),
        "issuer must be LOW even when neither endpoint equals the actual signer"
    );
    let mut self_pair = payload.clone();
    self_pair[43..75].copy_from_slice(low.device_id().as_bytes());
    assert!(SignedAdmitted::decode(&f::signed(
        7, &low, scope, [23; 32], 2000, 17000, &self_pair
    ))
    .is_err());
    let mut reversed = payload.clone();
    reversed[11..43].copy_from_slice(high.device_id().as_bytes());
    reversed[43..75].copy_from_slice(low.device_id().as_bytes());
    assert!(SignedAdmitted::decode(&f::signed(
        7, &high, scope, [23; 32], 2000, 17000, &reversed
    ))
    .is_err());
    for start in [267, 275] {
        let mut zero = payload.clone();
        zero[start..start + 8].fill(0);
        assert!(
            SignedAdmitted::decode(&f::signed(7, &low, scope, [23; 32], 2000, 17000, &zero))
                .is_err()
        );
    }
}

#[test]
fn exact_signed_agreement_and_commit_bind_all_hashes_nonces_and_endpoints() {
    let scope = f::scope(true);
    let cap = f::availability(scope, true);
    let request = f::request(scope, &cap);
    let request = SignedAdmitted::decode(&request).unwrap();
    let grant = f::grant(scope, request.hash(), 15000);
    let grant = SignedAdmitted::decode(&grant).unwrap();
    let a = assemble_agreement(&request, &grant).unwrap();
    assert_eq!(a.encode().unwrap().len(), 503);
    assert_eq!(a.request_digest, request.hash());
    assert_eq!(a.grant_digest, grant.hash());
    let (low, high) = f::identities();
    let mut tail = high.device_id().as_bytes().to_vec();
    for field in [
        a.request_digest,
        a.grant_digest,
        *RoundRef::derive(&a).unwrap().bytes(),
        a.low_nonce,
        a.high_nonce,
    ] {
        tail.extend(field);
    }
    let payload = f::payload(5, 7, &tail);
    let commit = f::signed(7, &low, scope, [25; 32], 4000, 19000, &payload);
    assert!(check_commit(&a, &SignedAdmitted::decode(&commit).unwrap()).is_ok());
    for offset in [32, 64, 96, 128, 160] {
        let mut bad = tail.clone();
        bad[offset] ^= 1;
        let signed = f::signed(
            7,
            &low,
            scope,
            [25; 32],
            4000,
            19000,
            &f::payload(5, 7, &bad),
        );
        assert!(check_commit(&a, &SignedAdmitted::decode(&signed).unwrap()).is_err());
    }
    let too_late = f::grant(scope, request.hash(), 16001);
    assert!(assemble_agreement(&request, &SignedAdmitted::decode(&too_late).unwrap()).is_err());
}

#[test]
fn grant_window_respects_exact_skew_saturation_and_upper_bound_without_cross_clock_ordering() {
    let mut h = common();
    for issued in [0, 1, 5000, 7000, u64::MAX - 1000] {
        h.issued = issued;
        h.expires = issued.saturating_add(1000);
        let lower = issued.saturating_sub(CLOCK_SKEW_MS);
        for bound in [lower, h.expires] {
            assert!(Grant {
                target: target(),
                request: [0; 32],
                high_nonce: [0; 32],
                commit_by_wall: bound
            }
            .check_window(h.issued, h.expires)
            .is_ok());
        }
        if lower != 0 {
            assert!(Grant {
                target: target(),
                request: [0; 32],
                high_nonce: [0; 32],
                commit_by_wall: lower - 1
            }
            .check_window(h.issued, h.expires)
            .is_err());
        }
        if h.expires != u64::MAX {
            assert!(Grant {
                target: target(),
                request: [0; 32],
                high_nonce: [0; 32],
                commit_by_wall: h.expires + 1
            }
            .check_window(h.issued, h.expires)
            .is_err());
        }
    }
}

#[test]
fn terminal_unknown_and_recorded_presence_reason_and_observation_matrix_is_closed() {
    let h = common();
    for decision in 0..=3 {
        for reason in 0..=6 {
            for grant in [false, true] {
                for round in [false, true] {
                    for at_wall in [0, 1] {
                        for observed in 0..=7 {
                            let payload = terminal(
                                &h,
                                7,
                                TerminalFields {
                                    decision,
                                    reason,
                                    grant,
                                    round,
                                    at_wall,
                                    observed,
                                },
                            );
                            let expected = match decision {
                                0 => {
                                    reason == 0 && !grant && !round && at_wall == 0 && observed == 6
                                }
                                1 => reason == 0 && grant && round && observed <= 5,
                                2 => {
                                    (1..=5).contains(&reason)
                                        && at_wall == 0
                                        && (!round || grant)
                                        && matches!(observed, 0 | 3 | 4 | 5)
                                }
                                _ => false,
                            };
                            let decoded = Control::decode(&payload, &h);
                            assert_eq!(decoded.is_ok(), expected,
                "decision{decision}/reason{reason}/GD{grant}/R{round}/at{at_wall}/observed{observed}");
                            if decision == 0 && expected {
                                assert!(matches!(
                                    decoded.unwrap().operation,
                                    Operation::Terminal(TerminalObservation::Unknown { .. })
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn immutable_terminal_digest_covers_record_fields_and_excludes_present_observations_and_wrapper_time(
) {
    let mut h = common();
    let payload = terminal(
        &h,
        7,
        TerminalFields {
            decision: 1,
            reason: 0,
            grant: true,
            round: true,
            at_wall: u64::MAX,
            observed: 1,
        },
    );
    assert_eq!(payload.len(), 184);
    for offset in 11..151 {
        // Every encoded immutable body field through accepted_wall.
        let mut bad = payload.clone();
        bad[offset] ^= 1;
        assert!(
            Control::decode(&bad, &h).is_err(),
            "immutable terminal byte {offset}"
        );
    }
    for observed in 0..=5 {
        let mut changed = payload.clone();
        changed[183] = observed;
        assert!(Control::decode(&changed, &h).is_ok());
    }
    h.id = [0; 32];
    h.issued = 0;
    h.expires = 1;
    assert!(
        Control::decode(&payload, &h).is_ok(),
        "historical wall is not reclassified by a later rollback"
    );
    for field in 0..5 {
        let mut changed = h;
        match field {
            0 => changed.scope.group[0] ^= 1,
            1 => changed.scope.policy = None,
            2 => changed.scope.policy.as_mut().unwrap()[0] ^= 1,
            3 => changed.scope.room ^= 1,
            _ => changed.issuer = DeviceKeypair::from_seed(&[3; 32]).device_id(),
        }
        assert!(
            Control::decode(&payload, &changed).is_err(),
            "immutable header context {field}"
        );
    }
    for offset in 3..11 {
        let mut changed = payload.clone();
        changed[offset] ^= 1;
        assert!(
            Control::decode(&changed, &h).is_err(),
            "immutable epoch byte {offset}"
        );
    }
    let mut unknown = terminal(
        &common(),
        7,
        TerminalFields {
            decision: 0,
            reason: 0,
            grant: false,
            round: false,
            at_wall: 0,
            observed: 6,
        },
    );
    unknown[11 + 65] = 1; // Nonzero absent GD bytes.
    assert!(Control::decode(&unknown, &common()).is_err());
}

#[test]
fn fixed_controls_reject_every_truncation_trailing_and_unknown_prefix() {
    let h = common();
    let mut tail = target().as_bytes().to_vec();
    tail.extend([1; 32]);
    let payload = f::payload(7, 7, &tail);
    assert_eq!(payload.len(), 75);
    for end in 0..payload.len() {
        assert!(Control::decode(&payload[..end], &h).is_err());
    }
    let mut trailing = payload.clone();
    trailing.push(0);
    assert!(Control::decode(&trailing, &h).is_err());
    for offset in [0, 1, 2] {
        let mut bad = payload.clone();
        bad[offset] = 255;
        assert!(Control::decode(&bad, &h).is_err());
    }
}
