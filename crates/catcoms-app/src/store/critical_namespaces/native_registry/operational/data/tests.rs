//! DATA grammar/semantic regressions. Arbitrary fixtures do not exercise native
//! authentication, SDK admission, physical commit, cold authoring or transport.
use super::super::super::super::archive_current_namespace_codec::CurrentLayoutData;
use super::*;

fn cap() -> Capability {
    let current = CurrentLayoutData {
        vault: [1; 32],
        root: [2; 32],
    };
    Capability::for_current_data(current, &current.encode_data().unwrap()).unwrap()
}
fn empty() -> Index {
    Index {
        capability: cap(),
        generation: 1,
        parent: None,
        last_issued_id: 0,
        rows: vec![],
    }
}
fn born() -> Index {
    Index {
        capability: cap(),
        generation: 2,
        parent: Some([9; 32]),
        last_issued_id: 1,
        rows: vec![Row {
            id: 1,
            identity: [10; 32],
            name: "original".into(),
            is_dm: false,
            backend: Backend::Pending {
                birth: Birth {
                    invocation: [3; 32],
                    request: [4; 32],
                    signer: [5; 32],
                    network: [6; 32],
                },
                network_frame: Zeroizing::new(vec![7; 40]),
            },
        }],
    }
}
fn clone_data(index: &Index) -> Index {
    Index::decode_data(&index.encode_data().unwrap(), index.capability).unwrap()
}
fn activated(index: &Index) -> Index {
    let mut next = clone_data(index);
    next.generation += 1;
    next.parent = Some([11; 32]);
    let birth = next.rows[0].backend.birth().unwrap().clone();
    next.rows[0].backend = Backend::Founder {
        birth: birth.clone(),
        binding: old::FounderBindingData {
            vault: index.capability.original.vault,
            layout: index.capability.original.root,
            group: vec![12; 32],
            signer: birth.signer,
            source: [13; 32],
            profile: [14; 32],
            pin: [15; 32],
        },
    };
    next
}

#[test]
fn native_index2_descriptor_and_index_are_closed_against_version1() {
    let cap = cap();
    let raw = cap.encode_data().unwrap();
    assert!(Capability::decode_exact(&raw, cap).is_ok());
    assert!(
        super::super::super::capability::NativeCapabilityData::decode_exact(&raw, cap.original)
            .is_err()
    );
    assert!(Capability::decode_exact(&cap.original.encode_data().unwrap(), cap).is_err());
    let index = born();
    let raw = index.encode_data().unwrap();
    assert!(old::IndexData::decode_data(&raw, cap.original).is_err());
    assert!(Index::decode_data(&raw, cap).is_ok());
    for end in 0..raw.len() {
        assert!(Index::decode_data(&raw[..end], cap).is_err());
    }
    let mut extra = raw.to_vec();
    extra.push(0);
    assert!(Index::decode_data(&extra, cap).is_err());
}

#[test]
fn native_index2_migration_preserves_genuine_g1_legacy_mapping_only() {
    let old = old::IndexData::migrate_legacy_data(
        cap().original,
        Some([20; 32]),
        &[crate::store::ServerRecord {
            id: 7,
            display_name: "old".into(),
            invite: "invite".into(),
            is_dm: true,
        }],
    )
    .unwrap();
    let identity = old.rows[0].identity;
    let next = Index::migrate_v1_data(old).unwrap();
    assert_eq!(next.rows[0].identity, identity);
    assert_eq!(next.last_issued_id, 7);
    assert!(matches!(&next.rows[0].backend, Backend::Legacy { invite } if invite == "invite"));
    let mut wrong = old::IndexData::migrate_legacy_data(cap().original, None, &[]).unwrap();
    wrong.generation = 2;
    wrong.parent = Some([1; 32]);
    assert!(Index::migrate_v1_data(wrong).is_err());
    let wrong = old::IndexData {
        capability: cap().original,
        generation: 1,
        parent: None,
        rows: vec![old::RowData {
            id: 0,
            identity: [2; 32],
            name: "pending".into(),
            is_dm: false,
            backend: old::BackendData::Pending(old::PendingCreateData {
                backend: old::CreationBackendData::FounderV2,
                invocation: [3; 32],
                signer: [4; 32],
                network: [5; 32],
            }),
        }],
    };
    assert!(Index::migrate_v1_data(wrong).is_err());
}

#[test]
fn native_index2_birth_requires_exact_parent_next_id_and_whole_prefix() {
    let initial = empty();
    let next = born();
    assert_eq!(
        initial.successor_data(&next, [9; 32]).unwrap(),
        Change::Birth
    );
    assert!(initial.successor_data(&next, [8; 32]).is_err());
    let mut changed = clone_data(&next);
    changed.rows[0].id = 2;
    changed.last_issued_id = 2;
    assert!(initial.successor_data(&changed, [9; 32]).is_err());
    let mut initial = empty();
    initial.last_issued_id = u64::MAX;
    assert!(initial.successor_data(&next, [9; 32]).is_err());
    let initial = born();
    let mut next = clone_data(&initial);
    next.generation += 1;
    next.parent = Some([16; 32]);
    next.last_issued_id = 2;
    let mut second = born().rows.remove(0);
    second.id = 2;
    second.identity = [17; 32];
    if let Backend::Pending { birth, .. } = &mut second.backend {
        birth.invocation = [18; 32];
    }
    next.rows.push(second);
    assert_eq!(
        initial.successor_data(&next, [16; 32]).unwrap(),
        Change::Birth
    );
    next.rows[0].name = "lost prior metadata".into();
    assert!(initial.successor_data(&next, [16; 32]).is_err());
}

#[test]
fn native_index2_receipts_survive_activation_retirement_and_abandonment() {
    let pending = born();
    let active = activated(&pending);
    assert_eq!(
        pending.successor_data(&active, [11; 32]).unwrap(),
        Change::Activate
    );
    let mut retired = clone_data(&active);
    retired.generation += 1;
    retired.parent = Some([21; 32]);
    let Backend::Founder { birth, binding } = &active.rows[0].backend else {
        unreachable!()
    };
    retired.rows[0].backend = Backend::Retired {
        birth: birth.clone(),
        binding: binding.clone(),
        scope_terminal: [22; 32],
    };
    assert_eq!(
        active.successor_data(&retired, [21; 32]).unwrap(),
        Change::Retire
    );
    let mut abandoned = clone_data(&pending);
    abandoned.generation += 1;
    abandoned.parent = Some([23; 32]);
    abandoned.rows[0].backend = Backend::Abandoned {
        birth: birth.clone(),
        reason: old::AbandonedCreateReasonData::BeforeSdk,
        journal_terminal: [24; 32],
    };
    assert_eq!(
        pending.successor_data(&abandoned, [23; 32]).unwrap(),
        Change::Abandon
    );
    for index in [&pending, &active, &retired, &abandoned] {
        let row = index.lookup_data([3; 32], [4; 32]).unwrap().unwrap();
        assert_eq!(row.identity, [10; 32]);
        assert!(index.lookup_data([3; 32], [99; 32]).is_err());
        assert!(index.lookup_data([99; 32], [4; 32]).unwrap().is_none());
    }
    let mut changed = clone_data(&retired);
    if let Backend::Retired { binding, .. } = &mut changed.rows[0].backend {
        binding.source = [99; 32];
    }
    assert!(active.successor_data(&changed, [21; 32]).is_err());
}

#[test]
fn native_index2_rejects_receipt_forgery_replacement_and_noop_successors() {
    let pending = born();
    let active = activated(&pending);
    let mut changed = clone_data(&active);
    if let Backend::Founder { birth, .. } = &mut changed.rows[0].backend {
        birth.request = [99; 32];
    }
    assert!(pending.successor_data(&changed, [11; 32]).is_err());
    let mut changed = clone_data(&active);
    changed.rows[0].identity = [99; 32];
    assert!(pending.successor_data(&changed, [11; 32]).is_err());
    let mut changed = clone_data(&pending);
    changed.generation += 1;
    changed.parent = Some([11; 32]);
    assert!(pending.successor_data(&changed, [11; 32]).is_err());
    let mut changed = clone_data(&active);
    if let Backend::Founder { binding, .. } = &mut changed.rows[0].backend {
        binding.signer = [99; 32];
    }
    assert!(changed.encode_data().is_err());
    let mut exhausted = clone_data(&pending);
    exhausted.generation = u64::MAX;
    assert!(exhausted.successor_data(&active, [11; 32]).is_err());
}

#[test]
fn native_index2_bounds_initial_network_frame_and_unique_receipts() {
    let mut index = born();
    for len in [0, 39, MAX_NETWORK_FRAME + 1] {
        if let Backend::Pending { network_frame, .. } = &mut index.rows[0].backend {
            *network_frame = Zeroizing::new(vec![0; len]);
        }
        assert!(index.encode_data().is_err());
    }
    if let Backend::Pending { network_frame, .. } = &mut index.rows[0].backend {
        *network_frame = Zeroizing::new(vec![0; MAX_NETWORK_FRAME]);
    }
    assert!(index.encode_data().is_ok());
    let mut second = born().rows.remove(0);
    second.id = 2;
    second.identity = [19; 32];
    index.rows.push(second);
    index.last_issued_id = 2;
    assert!(
        index.encode_data().is_err(),
        "duplicate token must refuse even with different row ID"
    );
    if let Backend::Pending { birth, .. } = &mut index.rows[1].backend {
        birth.invocation = [20; 32];
    }
    assert!(index.encode_data().is_ok());
    index.rows[1].identity = index.rows[0].identity;
    assert!(index.encode_data().is_err());
}

#[test]
fn native_index2_genesis_cannot_skip_birth_or_invent_an_issued_id() {
    let mut pending = born();
    pending.generation = 1;
    pending.parent = None;
    assert!(pending.encode_data().is_err());
    let mut active = activated(&born());
    active.generation = 1;
    active.parent = None;
    assert!(active.encode_data().is_err());
    let mut empty = empty();
    empty.last_issued_id = 99;
    assert!(empty.encode_data().is_err());
    // Malicious encoded bytes must also refuse, independently of the safe encoder.
    let mut raw = born().encode_data().unwrap();
    let generation_offset = 4 + TAG.len() + 2 + 3 * 36;
    raw[generation_offset..generation_offset + 8].copy_from_slice(&1_u64.to_be_bytes());
    // Remove the old parent rather than relying on a trivial framing mismatch.
    raw[generation_offset + 8] = 0;
    raw.drain(generation_offset + 9..generation_offset + 9 + 36);
    assert!(Index::decode_data(&raw, cap()).is_err());
}
