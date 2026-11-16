//! DATA-only codec/transition fixtures. No actual grants, journal barriers or tickets here.
//! All regressions authored UNRUN during the execution pause.
use super::*;

fn source() -> SourceIdentity {
    SourceIdentity {
        binding: [1; 32],
        request: [2; 32],
        package: [3; 32],
    }
}
fn grant() -> State {
    State {
        grant: 1,
        limit: 64,
        spent: 0,
        ordinal: 0,
        arm: None,
        input: None,
    }
}
fn arm(grant: u64, ordinal: u64, revision: u64) -> Arm {
    Arm {
        source: source(),
        grant,
        ordinal,
        scheduling_revision: revision,
        activation: [4; 32],
        invocation: [5; 32],
        deadline_ms: 1000,
    }
}
fn submitted() -> State {
    State {
        grant: 1,
        limit: 64,
        spent: 1,
        ordinal: 1,
        arm: Some(arm(1, 1, 1)),
        input: None,
    }
}
fn roundtrip(state: &State, input: Option<&Input<'_>>, revision: u64) -> Vec<u8> {
    let encoded = encode(state, input, &source(), revision).unwrap();
    let parsed = parse(&encoded, &source(), revision).unwrap();
    assert!(parsed.state == *state);
    assert_eq!(
        parsed.input.as_ref().map(|i| i.ciphertext),
        input.map(|i| i.ciphertext)
    );
    encoded
}

#[test]
fn strict_data_roundtrip_not_granted_grant_spent_and_received_older_arm() {
    roundtrip(&State::not_granted(), None, 1);
    roundtrip(&grant(), None, 1);
    roundtrip(&submitted(), None, 1);
    let nonce = [6; 24];
    let cipher = [7; 40];
    let summary = InputSummary {
        arm: arm(1, 1, 1),
        body_length: 8,
        body_digest: [8; 32],
        ciphertext_digest: cipher_digest(&nonce, &cipher),
    };
    let input = Input {
        summary,
        nonce: &nonce,
        ciphertext: &cipher,
    };
    let mut state = submitted();
    state.ordinal = 2;
    state.spent = 2;
    state.arm = Some(arm(1, 2, 1));
    state.input = Some(summary);
    roundtrip(&state, Some(&input), 1);
    let current_summary = InputSummary {
        arm: state.arm.unwrap(),
        ..summary
    };
    let current_input = Input {
        summary: current_summary,
        nonce: &nonce,
        ciphertext: &cipher,
    };
    state.input = Some(current_summary);
    roundtrip(&state, Some(&current_input), 1);
    // Cipher/framing equality is DATA only; it never establishes accepted selected-KP membership.
}

#[test]
fn truncated_trailing_bad_flags_nested_lengths_and_cross_source_refuse() {
    let encoded = roundtrip(&submitted(), None, 1);
    for n in 0..encoded.len() {
        assert!(parse(&encoded[..n], &source(), 1).is_err());
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(parse(&trailing, &source(), 1).is_err());
    let flag = 4 + TAG.len() + 8 + 4 + 4 + 8;
    let mut bad = encoded.clone();
    bad[flag] = 2;
    assert!(parse(&bad, &source(), 1).is_err());
    let mut bad = encoded;
    bad[flag + 1..flag + 5].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(parse(&bad, &source(), 1).is_err());
    let valid = roundtrip(&submitted(), None, 1);
    let mut wrong = source();
    wrong.package[0] ^= 1;
    assert!(parse(&valid, &wrong, 1).is_err());
    assert!(parse(&valid, &source(), 0).is_err());
}

#[test]
fn first_grant_and_debit_are_closed_and_exhaustion_or_counter_refund_refuses() {
    let original = State::not_granted();
    let first = grant();
    assert!(matches!(
        successor(&original, &first, &source(), 1, 1).unwrap(),
        Change::FirstGrant
    ));
    let spent = submitted();
    assert!(matches!(
        successor(&first, &spent, &source(), 1, 1).unwrap(),
        Change::Debit
    ));
    assert!(successor(&spent, &first, &source(), 1, 1).is_err());
    let mut full = spent;
    full.spent = 64;
    full.ordinal = 64;
    full.arm = Some(arm(1, 64, 1));
    let mut excess = full;
    excess.spent = 65;
    excess.ordinal = 65;
    excess.arm = Some(arm(1, 65, 1));
    assert!(successor(&full, &excess, &source(), 1, 1).is_err());
    let mut changed = spent;
    changed.limit = 63;
    assert!(successor(&first, &changed, &source(), 1, 1).is_err());
    let mut wrong = spent;
    wrong.arm.as_mut().unwrap().scheduling_revision = 2;
    assert!(successor(&first, &wrong, &source(), 1, 1).is_err());
}

#[test]
fn retry_grant_requires_new_saved_schedule_and_repeated_zero_submit_grants_keep_old_arm() {
    let first = submitted();
    let mut second = first;
    second.grant = 2;
    second.spent = 0;
    assert!(successor(&first, &second, &source(), 1, 1).is_err());
    assert!(matches!(
        successor(&first, &second, &source(), 1, 2).unwrap(),
        Change::RetryGrant
    ));
    let mut third = second;
    third.grant = 3;
    assert!(matches!(
        successor(&second, &third, &source(), 2, 3).unwrap(),
        Change::RetryGrant
    ));
    assert!(
        third.arm == first.arm,
        "ANY earlier genuine grant arm, not only current-1"
    );
    roundtrip(&third, None, 3);
    let mut debit = third;
    debit.spent = 1;
    debit.ordinal = 2;
    debit.arm = Some(arm(3, 2, 3));
    assert!(matches!(
        successor(&third, &debit, &source(), 3, 3).unwrap(),
        Change::Debit
    ));
    let mut exhausted = third;
    exhausted.grant = u64::MAX;
    let mut wrapping = exhausted;
    wrapping.grant = 0;
    assert!(successor(&exhausted, &wrapping, &source(), 3, 4).is_err());
    assert!(successor(&third, &debit, &source(), u64::MAX, u64::MAX).is_err());
}

#[test]
fn input_cannot_be_overwritten_cleared_or_buy_counter_units_and_old_arm_keeps_current_latest() {
    let mut old = submitted();
    old.spent = 2;
    old.ordinal = 2;
    old.arm = Some(arm(1, 2, 1));
    let nonce = [9; 24];
    let cipher = [10; 32];
    let summary = InputSummary {
        arm: arm(1, 1, 1),
        body_length: 4,
        body_digest: [11; 32],
        ciphertext_digest: cipher_digest(&nonce, &cipher),
    };
    let input = Input {
        summary,
        nonce: &nonce,
        ciphertext: &cipher,
    };
    let mut next = old;
    next.input = Some(summary);
    assert!(matches!(
        successor(&old, &next, &source(), 1, 1).unwrap(),
        Change::Input
    ));
    assert!(next.arm == old.arm);
    roundtrip(&next, Some(&input), 1);
    assert!(successor(&next, &old, &source(), 1, 1).is_err());
    let mut overwritten = next;
    overwritten.input.as_mut().unwrap().body_digest[0] ^= 1;
    assert!(successor(&next, &overwritten, &source(), 1, 1).is_err());
    let mut refund = next;
    refund.spent = 1;
    assert!(successor(&old, &refund, &source(), 1, 1).is_err());
    let mut future = next;
    future.input.as_mut().unwrap().arm.ordinal = 3;
    let future_input = Input {
        summary: future.input.unwrap(),
        nonce: &nonce,
        ciphertext: &cipher,
    };
    assert!(encode(&future, Some(&future_input), &source(), 1).is_err());
    assert!(successor(&old, &future, &source(), 1, 1).is_err());
    let mut raw_future = encode(&next, Some(&input), &source(), 1).unwrap();
    let mut arm_bytes = Encoder::new();
    encode_arm(&summary.arm, &mut arm_bytes).unwrap();
    let arm_bytes = arm_bytes.finish();
    let offset = raw_future
        .windows(arm_bytes.len())
        .rposition(|frame| frame == arm_bytes.as_slice())
        .unwrap();
    let ordinal_offset = offset + 3 * 36 + 8;
    raw_future[ordinal_offset..ordinal_offset + 8].copy_from_slice(&3u64.to_be_bytes());
    assert!(parse(&raw_future, &source(), 1).is_err());
    let mut bad_nonce = nonce;
    bad_nonce[0] ^= 1;
    let changed = Input {
        summary,
        nonce: &bad_nonce,
        ciphertext: &cipher,
    };
    assert!(encode(&next, Some(&changed), &source(), 1).is_err());
}

#[test]
fn input_equal_ordinal_requires_exact_arm_and_older_arm_cannot_postdate_latest_grant_or_schedule() {
    let mut original = submitted();
    original.spent = 2;
    original.ordinal = 2;
    original.arm = Some(arm(1, 2, 1));
    // Two subsequent zero-submit grants preserve the original latest arm at grant1/revision1.
    original.grant = 3;
    original.spent = 0;
    let nonce = [15; 24];
    let cipher = [16; 32];
    for variant in 0..7 {
        let mut impossible = original.arm.unwrap();
        match variant {
            0 => impossible.activation[0] ^= 1,
            1 => impossible.invocation[0] ^= 1,
            2 => impossible.deadline_ms += 1,
            3 => impossible.scheduling_revision = 2,
            4 => {
                impossible.ordinal = 1;
                impossible.grant = 2;
            }
            5 => {
                impossible.ordinal = 1;
                impossible.scheduling_revision = 2;
            }
            6 => impossible.grant = 2,
            _ => unreachable!(),
        }
        let summary = InputSummary {
            arm: impossible,
            body_length: 4,
            body_digest: [17; 32],
            ciphertext_digest: cipher_digest(&nonce, &cipher),
        };
        let input = Input {
            summary,
            nonce: &nonce,
            ciphertext: &cipher,
        };
        let mut bad = original;
        bad.input = Some(summary);
        assert!(encode(&bad, Some(&input), &source(), 3).is_err());
        assert!(successor(&original, &bad, &source(), 3, 3).is_err());
    }
    let summary = InputSummary {
        arm: arm(1, 1, 1),
        body_length: 4,
        body_digest: [17; 32],
        ciphertext_digest: cipher_digest(&nonce, &cipher),
    };
    let input = Input {
        summary,
        nonce: &nonce,
        ciphertext: &cipher,
    };
    let mut genuine_older = original;
    genuine_older.input = Some(summary);
    roundtrip(&genuine_older, Some(&input), 3);
    assert!(matches!(
        successor(&original, &genuine_older, &source(), 3, 3).unwrap(),
        Change::Input
    ));
}

#[test]
fn field_caps_and_zero_states_are_negative_not_implicit_initial_grants() {
    let mut zero = State::not_granted();
    zero.limit = 1;
    assert!(encode(&zero, None, &source(), 1).is_err());
    let mut excessive = grant();
    excessive.limit = MAX_GRANT + 1;
    assert!(encode(&excessive, None, &source(), 1).is_err());
    let mut overflow = submitted();
    overflow.ordinal = u64::MAX;
    overflow.arm.as_mut().unwrap().ordinal = u64::MAX;
    let mut next = overflow;
    next.spent += 1;
    assert!(successor(&overflow, &next, &source(), 1, 1).is_err());
    let nonce = [12; 24];
    let ciphertext = vec![13; MAX_INPUT_CIPHER + 1];
    let summary = InputSummary {
        arm: arm(1, 1, 1),
        body_length: MAX_BODY as u32,
        body_digest: [14; 32],
        ciphertext_digest: cipher_digest(&nonce, &ciphertext),
    };
    let input = Input {
        summary,
        nonce: &nonce,
        ciphertext: &ciphertext,
    };
    let mut state = submitted();
    state.input = Some(summary);
    assert!(encode(&state, Some(&input), &source(), 1).is_err());
}

#[test]
fn maximum_input_cipher_appends_exact_prepaid_length_and_roundtrips_positive_current_arm() {
    let nonce = [0x41; 24];
    let ciphertext = vec![0x42; MAX_INPUT_CIPHER];
    let summary = InputSummary {
        arm: arm(1, 1, 1),
        body_length: MAX_BODY as u32,
        body_digest: [0x43; 32],
        ciphertext_digest: cipher_digest(&nonce, &ciphertext),
    };
    let input = Input {
        summary,
        nonce: &nonce,
        ciphertext: &ciphertext,
    };
    let mut state = submitted();
    state.input = Some(summary);
    let expected = encoded_len(&state, Some(&input), &source(), 1).unwrap();
    let mut output = Encoder::with_capacity(expected);
    encode_into(&state, Some(&input), &source(), 1, &mut output).unwrap();
    assert_eq!(output.len(), expected);
    let bytes = output.finish();
    let decoded = parse(&bytes, &source(), 1).unwrap();
    assert!(decoded.state == state);
    assert_eq!(decoded.input.unwrap().ciphertext, ciphertext);
}
