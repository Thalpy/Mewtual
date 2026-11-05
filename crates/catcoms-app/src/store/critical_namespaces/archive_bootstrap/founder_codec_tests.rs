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
