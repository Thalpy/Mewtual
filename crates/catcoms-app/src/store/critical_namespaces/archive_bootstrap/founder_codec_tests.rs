//! Local DATA grammar/MAC fixtures only. These labels are not Fresh, saved-root or physical
//! creation/arm tickets. Genuine physical founder/Store tests belong to the actual host consumer.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
const KEY: [u8; 32] = [77; 32];
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::from_seed([78; 32])
}
fn debit() -> DebitData {
    DebitData {
        invocation: InvocationData {
            vault: [1; 32],
            root: [2; 32],
            invocation: [3; 32],
        },
        signer: [4; 32],
        name: "n".repeat(256),
        limits: FounderSnapshotLimits::new(
            8 * 1024 * 1024 + 64 * 1024,
            8 * 1024 * 1024,
            8 * 1024 * 1024,
        )
        .unwrap(),
        io_allowance: 8,
    }
}
fn facts() -> FoundingData {
    FoundingData {
        group: vec![5; 256],
        signer: [4; 32],
        fresh_attempt: [6; 32],
        scope_domain: [1; 32],
        nonce: [7; 32],
        revision: 1,
    }
}
fn marker(debit: &DebitData, id: DebitId) -> TargetMarkerData {
    TargetMarkerData {
        invocation: debit.invocation,
        debit: id,
        founding: facts(),
    }
}
fn context(debit: &DebitData) -> BootstrapSourceContextData {
    let f = facts();
    BootstrapSourceContextData::from_data(BootstrapSourceContextSpec {
        vault: debit.invocation.vault,
        group: &f.group,
        signer: f.signer,
        fresh_attempt: f.fresh_attempt,
        scope_domain: f.scope_domain,
        scope_nonce: f.nonce,
        planned_revision: 1,
        profile: OwnerFactId::from_bytes([8; 32]),
        admission: OwnerBodyId::from_bytes([9; 32]),
        header: OwnerHeaderId::from_bytes([10; 32]),
        records: PrivateChatPageId::from_bytes([11; 32]),
        manifest: PrivateChatPageId::from_bytes([12; 32]),
        limits: debit.limits,
        snapshot_digest: [13; 32],
        records_plaintext_bytes: 65_536,
    })
    .unwrap()
}
fn first(debit: &DebitData, id: DebitId, marker: TargetMarkerId) -> IntentData {
    IntentData {
        invocation: debit.invocation,
        debit: id,
        sequence: 1,
        parent: None,
        io_spent: 0,
        kind: IntentKind::Founding {
            facts: facts(),
            marker,
        },
    }
}
fn source(debit: &DebitData, id: DebitId, marker: TargetMarkerId, parent: IntentId) -> IntentData {
    IntentData {
        invocation: debit.invocation,
        debit: id,
        sequence: 2,
        parent: Some(parent),
        io_spent: 0,
        kind: IntentKind::SourcePrepared {
            context: context(debit),
            source: BootstrapSourceId::from_bytes([14; 32]),
            marker,
        },
    }
}

#[test]
fn maximum_data_grammar_reaches_exact_debit_marker_context_intent_and_seed_sizes() {
    let d = debit();
    assert_eq!(d.encode().unwrap().len(), 488);
    let df = seal_debit_data(&KEY, &d, &mut rng()).unwrap();
    let did = DebitId::of(&df);
    assert!(open_debit_data(&KEY, d.invocation, &df).unwrap() == d);
    let m = marker(&d, did);
    assert_eq!(m.encode(&d, did).unwrap().len(), 600);
    let mf = seal_target_marker_data(&KEY, &m, &d, did, &mut rng()).unwrap();
    let mid = TargetMarkerId::of(&mf);
    let first = first(&d, did, mid);
    let ff = seal_intent_data(&KEY, &first, &d, did, &mut rng()).unwrap();
    let sp = source(&d, did, mid, IntentId::of(&ff));
    assert_eq!(encode_context(&context(&d)).unwrap().len(), 692);
    assert_eq!(sp.encode(&d, did).unwrap().len(), 1008);
    let sf = seal_intent_data(&KEY, &sp, &d, did, &mut rng()).unwrap();
    let seed =
        encode_transition_seed(&KEY, &d, did, Some(IntentId::of(&ff)), Some(&mf), &sf).unwrap();
    // Actual encodings are below the caps:1K intent body +40B, marker600+40B.
    assert!(seed.len() <= 2366);
    let sealed = seal_transition_seed(&KEY, &seed, &mut rng()).unwrap();
    let plain = open_transition_seed(&KEY, &sealed).unwrap();
    let parsed = parse_transition_seed_data(&KEY, &plain, &d, did).unwrap();
    assert!(parsed.candidate == sp);
    assert!(parsed.marker.as_ref() == Some(&m));
}

#[test]
fn original_request_binds_full_device_signer_name_limits_allowance_and_fixed_invocation() {
    let d = debit();
    let old = d.request_binding();
    let mut changed = d.clone();
    changed.signer[0] ^= 1;
    assert_ne!(changed.request_binding(), old);
    changed = d.clone();
    changed.name.push('x');
    assert!(changed.validate().is_err());
    changed = d.clone();
    changed.name.replace_range(..1, "x");
    assert_ne!(changed.request_binding(), old);
    changed = d.clone();
    changed.io_allowance = 7;
    assert_ne!(changed.request_binding(), old);
    changed = d.clone();
    changed.limits = FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap();
    assert_ne!(changed.request_binding(), old);
    let frame = seal_debit_data(&KEY, &d, &mut rng()).unwrap();
    for at in 0..3 {
        let mut expected = d.invocation;
        match at {
            0 => expected.vault[0] ^= 1,
            1 => expected.root[0] ^= 1,
            _ => expected.invocation[0] ^= 1,
        };
        assert!(open_debit_data(&KEY, expected, &frame).is_err());
    }
    assert!(open_debit_data(&[0; 32], d.invocation, &frame).is_err());
    assert_ne!(
        d.invocation.name(),
        InvocationData {
            root: [0; 32],
            ..d.invocation
        }
        .name()
    );
}

#[test]
fn strict_version_eof_length_identity_and_cumulative_counter_checks() {
    let d = debit();
    let raw = d.encode().unwrap();
    for at in [0, 4, 4 + DEBIT_TAG.len()] {
        let mut bad = raw.to_vec();
        bad[at] ^= 1;
        assert!(DebitData::decode(&bad, d.invocation).is_err());
    }
    assert!(DebitData::decode(&[raw.as_slice(), &[0]].concat(), d.invocation).is_err());
    assert!(DebitData::decode(&raw[..raw.len() - 1], d.invocation).is_err());
    for io in [0, 9, u32::MAX] {
        let mut invalid = d.clone();
        invalid.io_allowance = io;
        assert!(invalid.encode().is_err());
    }
    let df = seal_debit_data(&KEY, &d, &mut rng()).unwrap();
    let did = DebitId::of(&df);
    let mf = seal_target_marker_data(&KEY, &marker(&d, did), &d, did, &mut rng()).unwrap();
    let mid = TargetMarkerId::of(&mf);
    let first = first(&d, did, mid);
    let ff = seal_intent_data(&KEY, &first, &d, did, &mut rng()).unwrap();
    let mut sp = source(&d, did, mid, IntentId::of(&ff));
    for io in [1, 9, u32::MAX] {
        sp.io_spent = io;
        assert!(sp.encode(&d, did).is_err());
    }
    sp.io_spent = 0;
    sp.sequence = u64::MAX;
    assert!(sp.encode(&d, did).is_err());
    sp.sequence = 2;
    sp.parent = None;
    assert!(sp.encode(&d, did).is_err());
}

#[test]
fn exact_founder_to_source_and_arm_data_transitions_preserve_complete_originals() {
    let d = debit();
    let did = DebitId::from_bytes([21; 32]);
    let mid = TargetMarkerId::from_bytes([22; 32]);
    let first = first(&d, did, mid);
    first.successor(None, &d, did).unwrap();
    let parent = IntentId::from_bytes([23; 32]);
    let sp = source(&d, did, mid, parent);
    sp.successor(Some((&first, parent)), &d, did).unwrap();
    let mut arm = sp.clone();
    arm.sequence = 3;
    arm.io_spent = 1;
    arm.parent = Some(IntentId::from_bytes([24; 32]));
    arm.successor(Some((&sp, IntentId::from_bytes([24; 32]))), &d, did)
        .unwrap();
    let mut changed = arm.clone();
    if let IntentKind::SourcePrepared { source, .. } = &mut changed.kind {
        *source = BootstrapSourceId::from_bytes([25; 32]);
    }
    assert!(changed
        .successor(Some((&sp, IntentId::from_bytes([24; 32]))), &d, did)
        .is_err());
    changed = arm.clone();
    if let IntentKind::SourcePrepared { context, .. } = &mut changed.kind {
        let mut spec = context.spec_data();
        spec.scope_nonce[0] ^= 1;
        *context = BootstrapSourceContextData::from_data(spec).unwrap();
    }
    assert!(changed
        .successor(Some((&sp, IntentId::from_bytes([24; 32]))), &d, did)
        .is_err());
    assert!(arm
        .successor(Some((&sp, IntentId::from_bytes([99; 32]))), &d, did)
        .is_err());
    assert!(sp.successor(None, &d, did).is_err());
}

#[test]
fn no_source_prepared_to_closed_or_arm_reset_from_missing_source_labels() {
    let d = debit();
    let did = DebitId::from_bytes([31; 32]);
    let mid = TargetMarkerId::from_bytes([32; 32]);
    let old = IntentId::from_bytes([33; 32]);
    let sp = source(&d, did, mid, old);
    let close = IntentData {
        invocation: d.invocation,
        debit: did,
        sequence: 2,
        parent: Some(old),
        io_spent: 0,
        kind: IntentKind::ClosedFounding {
            facts: facts(),
            marker: mid,
            reason: CloseReason::CreatorLost,
        },
    };
    assert!(close.successor(Some((&sp, old)), &d, did).is_err());
    let mut reset = sp.clone();
    reset.parent = Some(old);
    assert!(reset.successor(Some((&sp, old)), &d, did).is_err());
}

#[test]
fn target_marker_requires_exact_full_signer_scope_revision_and_independent_debit() {
    let d = debit();
    let did = DebitId::from_bytes([41; 32]);
    let good = marker(&d, did);
    for n in 0..5 {
        let mut bad = good.clone();
        match n {
            0 => bad.founding.signer[0] ^= 1,
            1 => bad.founding.scope_domain[0] ^= 1,
            2 => bad.founding.revision = 2,
            3 => bad.debit = DebitId::from_bytes([0; 32]),
            _ => bad.invocation.root[0] ^= 1,
        };
        assert!(bad.encode(&d, did).is_err());
    }
    let raw = good.encode(&d, did).unwrap();
    assert!(TargetMarkerData::decode(&[raw.as_slice(), &[0]].concat(), &d, did).is_err());
}

#[test]
fn seed_validates_actual_ciphertext_context_parent_and_marker_not_separate_dto() {
    let d = debit();
    let df = seal_debit_data(&KEY, &d, &mut rng()).unwrap();
    let did = DebitId::of(&df);
    let mf = seal_target_marker_data(&KEY, &marker(&d, did), &d, did, &mut rng()).unwrap();
    let mid = TargetMarkerId::of(&mf);
    let first = first(&d, did, mid);
    let ff = seal_intent_data(&KEY, &first, &d, did, &mut rng()).unwrap();
    assert!(encode_transition_seed(&KEY, &d, did, None, None, &ff).is_err());
    assert!(encode_transition_seed(
        &KEY,
        &d,
        did,
        Some(IntentId::from_bytes([0; 32])),
        Some(&mf),
        &ff
    )
    .is_err());
    let mut corrupt = ff.to_vec();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert!(encode_transition_seed(&KEY, &d, did, None, Some(&mf), &corrupt).is_err());
    let mut foreign = marker(&d, did);
    foreign.founding.nonce[0] ^= 1;
    let foreign = seal_target_marker_data(&KEY, &foreign, &d, did, &mut rng()).unwrap();
    assert!(encode_transition_seed(&KEY, &d, did, None, Some(&foreign), &ff).is_err());
}

#[test]
fn debit_only_closure_seed_has_no_target_and_cross_kind_keys_never_alias() {
    let d = debit();
    let df = seal_debit_data(&KEY, &d, &mut rng()).unwrap();
    let did = DebitId::of(&df);
    let close = IntentData {
        invocation: d.invocation,
        debit: did,
        sequence: 1,
        parent: None,
        io_spent: 0,
        kind: IntentKind::ClosedDebit(CloseReason::CreatorLost),
    };
    let cf = seal_intent_data(&KEY, &close, &d, did, &mut rng()).unwrap();
    let raw = encode_transition_seed(&KEY, &d, did, None, None, &cf).unwrap();
    let parsed = parse_transition_seed_data(&KEY, &raw, &d, did).unwrap();
    assert!(parsed.marker.is_none());
    assert!(open_debit_data(&KEY, d.invocation, &cf).is_err());
    assert!(open_intent_data(&KEY, &d, did, &df).is_err());
    let marker = seal_target_marker_data(&KEY, &marker(&d, did), &d, did, &mut rng()).unwrap();
    assert!(encode_transition_seed(&KEY, &d, did, None, Some(&marker), &cf).is_err());
}

/// Local DATA/MAC fixture only. A valid encoded terminal is deliberately NOT a real completed
/// worker, saved arm or SavedSourceClosure. Root's physical tests must supply those originals.
struct TerminalFixture {
    debit: DebitData,
    debit_id: DebitId,
    marker_frame: Zeroizing<Vec<u8>>,
    arm: IntentData,
    arm_frame: Zeroizing<Vec<u8>>,
    terminal: IntentData,
}
fn terminal_fixture(spent: u32) -> TerminalFixture {
    let debit = debit();
    let debit_frame = seal_debit_data(&KEY, &debit, &mut rng()).unwrap();
    let debit_id = DebitId::of(&debit_frame);
    let marker_frame = seal_target_marker_data(
        &KEY,
        &marker(&debit, debit_id),
        &debit,
        debit_id,
        &mut rng(),
    )
    .unwrap();
    let marker_id = TargetMarkerId::of(&marker_frame);
    let mut arm = source(&debit, debit_id, marker_id, IntentId::from_bytes([81; 32]));
    arm.sequence = 2 + u64::from(spent);
    arm.io_spent = spent;
    let arm_frame = seal_intent_data(&KEY, &arm, &debit, debit_id, &mut rng()).unwrap();
    let IntentKind::SourcePrepared {
        context,
        source,
        marker,
    } = arm.kind.clone()
    else {
        unreachable!();
    };
    let terminal = IntentData {
        invocation: debit.invocation,
        debit: debit_id,
        sequence: arm.sequence + 1,
        parent: Some(IntentId::of(&arm_frame)),
        io_spent: spent,
        kind: IntentKind::SourceSaved {
            context,
            source,
            marker,
        },
    };
    TerminalFixture {
        debit,
        debit_id,
        marker_frame,
        arm,
        arm_frame,
        terminal,
    }
}

/// Locate only fixed common framing in an already encoded fixture; malformed-field tests
/// then exercise the real decoder, rather than calling a validation helper with a status flag.
fn intent_kind_and_payload_offsets(raw: &[u8]) -> (usize, usize) {
    let mut d = Decoder::new(raw);
    assert!(d.get_bytes().unwrap() == INTENT_TAG);
    assert_eq!(d.get_u16().unwrap(), VERSION);
    get_invocation(&mut d).unwrap();
    digest(&mut d).unwrap();
    d.get_u64().unwrap();
    get_parent(&mut d).unwrap();
    let kind = raw.len() - d.remaining();
    d.get_u8().unwrap();
    d.get_u32().unwrap();
    (kind, raw.len() - d.remaining())
}

#[test]
fn source_saved_first_and_exhausted_pass_keep_original_spent_units() {
    for spent in [1, 8] {
        let f = terminal_fixture(spent);
        f.terminal
            .successor(
                Some((&f.arm, IntentId::of(&f.arm_frame))),
                &f.debit,
                f.debit_id,
            )
            .unwrap();
        let frame = seal_intent_data(&KEY, &f.terminal, &f.debit, f.debit_id, &mut rng()).unwrap();
        let decoded = open_intent_data(&KEY, &f.debit, f.debit_id, &frame).unwrap();
        assert!(decoded == f.terminal);
        assert_eq!(decoded.io_spent, spent);
        assert_eq!(decoded.sequence, 3 + u64::from(spent));
        assert_eq!(decoded.parent, Some(IntentId::of(&f.arm_frame)));
        assert!(matches!(decoded.kind, IntentKind::SourceSaved { .. }));
        // Equality with allowance is a valid terminal, not a request for another write unit.
        if spent == 8 {
            assert_eq!(decoded.io_spent, f.debit.io_allowance);
        }
    }
}

#[test]
fn source_saved_preserves_maximum_payload_geometry_and_existing_kind_tags() {
    let f = terminal_fixture(8);
    let terminal = f.terminal.encode(&f.debit, f.debit_id).unwrap();
    let arm = f.arm.encode(&f.debit, f.debit_id).unwrap();
    assert_eq!(terminal.len(), 1008);
    assert_eq!(arm.len(), 1008);
    let (kind_at, payload_at) = intent_kind_and_payload_offsets(&terminal);
    let (arm_kind_at, arm_payload_at) = intent_kind_and_payload_offsets(&arm);
    assert_eq!(terminal[kind_at], 5);
    assert_eq!(arm[arm_kind_at], 2);
    assert_eq!(payload_at, arm_payload_at);
    assert!(terminal[payload_at..] == arm[arm_payload_at..]);
    assert_eq!(terminal[payload_at], 0, "closed payload version");

    let marker_id = TargetMarkerId::of(&f.marker_frame);
    let founding = first(&f.debit, f.debit_id, marker_id);
    let closed_debit = IntentData {
        invocation: f.debit.invocation,
        debit: f.debit_id,
        sequence: 1,
        parent: None,
        io_spent: 0,
        kind: IntentKind::ClosedDebit(CloseReason::CreatorLost),
    };
    let closed_founding = IntentData {
        sequence: 2,
        parent: Some(IntentId::from_bytes([82; 32])),
        kind: IntentKind::ClosedFounding {
            facts: facts(),
            marker: marker_id,
            reason: CloseReason::CreatorLost,
        },
        ..closed_debit.clone()
    };
    for (legacy, tag) in [
        (founding, 1),
        (f.arm, 2),
        (closed_debit, 3),
        (closed_founding, 4),
    ] {
        let raw = legacy.encode(&f.debit, f.debit_id).unwrap();
        let (at, _) = intent_kind_and_payload_offsets(&raw);
        assert_eq!(raw[at], tag);
        assert!(IntentData::decode(&raw, &f.debit, f.debit_id).unwrap() == legacy);
    }
}

#[test]
fn source_saved_strict_framing_version_and_counters_refuse_before_adoption() {
    let f = terminal_fixture(1);
    let raw = f.terminal.encode(&f.debit, f.debit_id).unwrap();
    let (kind_at, payload_at) = intent_kind_and_payload_offsets(&raw);
    for tag in [0, 6, u8::MAX] {
        let mut bad = raw.to_vec();
        bad[kind_at] = tag;
        assert!(IntentData::decode(&bad, &f.debit, f.debit_id).is_err());
    }
    let mut bad_payload_version = raw.to_vec();
    bad_payload_version[payload_at] = 1;
    assert!(IntentData::decode(&bad_payload_version, &f.debit, f.debit_id).is_err());
    let mut bad_outer_version = raw.to_vec();
    bad_outer_version[4 + INTENT_TAG.len() + 1] ^= 1;
    assert!(IntentData::decode(&bad_outer_version, &f.debit, f.debit_id).is_err());
    assert!(IntentData::decode(&[raw.as_slice(), &[0]].concat(), &f.debit, f.debit_id).is_err());
    for length in [0, kind_at, payload_at, raw.len() - 1] {
        assert!(IntentData::decode(&raw[..length], &f.debit, f.debit_id).is_err());
    }
    for spent in [0, 9, u32::MAX] {
        let mut bad = f.terminal.clone();
        bad.io_spent = spent;
        bad.sequence = 3 + u64::from(spent);
        assert!(bad.encode(&f.debit, f.debit_id).is_err());
    }
    for sequence in [0, 2, 3, 5, u64::MAX] {
        let mut bad = f.terminal.clone();
        bad.sequence = sequence;
        assert!(bad.encode(&f.debit, f.debit_id).is_err());
    }
    let mut no_parent = f.terminal.clone();
    no_parent.parent = None;
    assert!(no_parent.encode(&f.debit, f.debit_id).is_err());
}

#[test]
fn source_saved_has_no_outgoing_transition_or_counter_reset() {
    let f = terminal_fixture(1);
    let terminal_id = IntentId::from_bytes([83; 32]);
    // Both candidates are independently well-formed and have the next sequence. Failure
    // must follow from terminal finality, not merely malformed counters or a missing parent.
    let mut reopen = f.arm.clone();
    reopen.parent = Some(terminal_id);
    reopen.io_spent = 3;
    reopen.sequence = 5;
    reopen.validate(&f.debit, f.debit_id).unwrap();
    assert!(reopen
        .successor(Some((&f.terminal, terminal_id)), &f.debit, f.debit_id)
        .is_err());
    let mut repeated = f.terminal.clone();
    repeated.parent = Some(terminal_id);
    repeated.io_spent = 2;
    repeated.sequence = 5;
    repeated.validate(&f.debit, f.debit_id).unwrap();
    assert!(repeated
        .successor(Some((&f.terminal, terminal_id)), &f.debit, f.debit_id)
        .is_err());
    assert!(f.terminal.successor(None, &f.debit, f.debit_id).is_err());
    let founding = first(&f.debit, f.debit_id, TargetMarkerId::of(&f.marker_frame));
    assert!(f
        .terminal
        .successor(
            Some((&founding, IntentId::of(&f.arm_frame))),
            &f.debit,
            f.debit_id,
        )
        .is_err());
}

#[test]
fn source_saved_requires_exact_actual_parent_and_complete_original_binding() {
    let f = terminal_fixture(1);
    let parent = IntentId::of(&f.arm_frame);
    for mutation in 0..5 {
        let mut changed = f.terminal.clone();
        match mutation {
            0 => changed.parent = Some(IntentId::from_bytes([84; 32])),
            1 => {
                let IntentKind::SourceSaved { source, .. } = &mut changed.kind else {
                    unreachable!();
                };
                *source = BootstrapSourceId::from_bytes([85; 32]);
            }
            2 => {
                let IntentKind::SourceSaved { marker, .. } = &mut changed.kind else {
                    unreachable!();
                };
                *marker = TargetMarkerId::from_bytes([86; 32]);
            }
            3 => {
                let IntentKind::SourceSaved { context, .. } = &mut changed.kind else {
                    unreachable!();
                };
                let old = context.clone();
                let mut spec = old.spec_data();
                spec.profile = OwnerFactId::from_bytes([87; 32]);
                *context = BootstrapSourceContextData::from_data(spec).unwrap();
            }
            4 => {
                changed.io_spent = 2;
                changed.sequence = 5;
            }
            _ => unreachable!(),
        }
        // Each is legal standalone DATA; none may replace the selected actual SP.
        changed.validate(&f.debit, f.debit_id).unwrap();
        assert!(changed
            .successor(Some((&f.arm, parent)), &f.debit, f.debit_id)
            .is_err());
    }
    let mut foreign_debit = f.debit.clone();
    foreign_debit.invocation.invocation[0] ^= 1;
    assert!(f.terminal.encode(&foreign_debit, f.debit_id).is_err());
    assert!(f
        .terminal
        .encode(&f.debit, DebitId::from_bytes([88; 32]))
        .is_err());
}

#[test]
fn source_saved_seed_parent_label_is_not_the_actual_parent_transition_proof() {
    let f = terminal_fixture(8);
    let parent = IntentId::of(&f.arm_frame);
    for retarget in [false, true] {
        let mut candidate = f.terminal.clone();
        if retarget {
            let IntentKind::SourceSaved { source, .. } = &mut candidate.kind else {
                unreachable!();
            };
            *source = BootstrapSourceId::from_bytes([89; 32]);
        }
        let frame = seal_intent_data(&KEY, &candidate, &f.debit, f.debit_id, &mut rng()).unwrap();
        let seed = encode_transition_seed(
            &KEY,
            &f.debit,
            f.debit_id,
            Some(parent),
            Some(&f.marker_frame),
            &frame,
        )
        .unwrap();
        assert!(seed.len() <= 2366);
        let parsed = parse_transition_seed_data(&KEY, &seed, &f.debit, f.debit_id).unwrap();
        assert_eq!(parsed.expected_parent, Some(parent));
        assert!(parsed.candidate == candidate);
        // Complete outer MAC/marker/parent-label consistency does NOT prove same SourceId.
        // The actual writer must perform this additional comparison with the selected parent.
        assert_eq!(
            parsed
                .candidate
                .successor(Some((&f.arm, parent)), &f.debit, f.debit_id)
                .is_ok(),
            !retarget
        );
    }
}

#[test]
fn source_saved_debug_omits_private_context_and_correlation_labels() {
    let f = terminal_fixture(1);
    assert_eq!(format!("{:?}", f.terminal), "FounderIntentData { .. }");
}
