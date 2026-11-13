//! Grammar fixtures only. No arbitrary DATA below supplies a genuine native owner.
use super::super::super::archive_current_namespace_codec::CurrentLayoutData;
use super::*;

fn capability() -> NativeCapabilityData {
    let layout = CurrentLayoutData {
        vault: [1; 32],
        root: [2; 32],
    };
    NativeCapabilityData::for_current_data(layout, &layout.encode_data().unwrap()).unwrap()
}
fn pending() -> PendingCreateData {
    PendingCreateData {
        backend: CreationBackendData::FounderV2,
        invocation: [3; 32],
        signer: [4; 32],
        network: [5; 32],
    }
}
fn binding(cap: NativeCapabilityData) -> FounderBindingData {
    FounderBindingData {
        vault: cap.vault,
        layout: cap.root,
        group: b"grammar-only group".to_vec(),
        signer: [4; 32],
        source: [6; 32],
        profile: [7; 32],
        pin: [8; 32],
    }
}
fn mixed() -> IndexData {
    let cap = capability();
    let backends = [
        BackendData::Legacy {
            invite: "legacy invite".into(),
        },
        BackendData::Pending(pending()),
        BackendData::Founder(binding(cap)),
        BackendData::RetiredFounder {
            binding: binding(cap),
            scope_terminal: [9; 32],
        },
        BackendData::AbandonedCreate {
            pending: pending(),
            reason: AbandonedCreateReasonData::UnexposedAfterSdk,
            journal_terminal: [10; 32],
        },
    ];
    IndexData {
        capability: cap,
        generation: 1,
        parent: None,
        rows: backends
            .into_iter()
            .enumerate()
            .map(|(i, backend)| RowData {
                id: i as u64,
                identity: [i as u8 + 20; 32],
                name: format!("row {i}"),
                is_dm: i == 0,
                backend,
            })
            .collect(),
    }
}
fn empty() -> IndexData {
    IndexData {
        rows: vec![],
        ..mixed()
    }
}
fn header_length() -> usize {
    empty().encode_data().unwrap().len()
}
fn compact_legacy_rows() -> IndexData {
    let mut value = mixed();
    value.rows.truncate(2);
    for row in &mut value.rows {
        row.name.clear();
        row.is_dm = false;
        row.backend = BackendData::Legacy {
            invite: String::new(),
        };
    }
    value
}

#[test]
fn native_data_all_known_states_roundtrip_without_role_conversion() {
    let value = mixed();
    let encoded = value.encode_data().unwrap();
    assert_eq!(
        IndexData::decode_data(&encoded, value.capability).unwrap(),
        value
    );
    assert_eq!(
        IndexData::decode_data(&empty().encode_data().unwrap(), capability()).unwrap(),
        empty()
    );
}

#[test]
fn native_data_all_prefix_cuts_and_trailing_bytes_refuse_whole_snapshot() {
    let value = mixed();
    let raw = value.encode_data().unwrap();
    for end in 0..raw.len() {
        assert!(
            IndexData::decode_data(&raw[..end], value.capability).is_err(),
            "cut {end}"
        );
    }
    let mut extra = raw.to_vec();
    extra.push(0);
    assert!(IndexData::decode_data(&extra, value.capability).is_err());
}

#[test]
fn native_data_unknown_versions_roles_and_booleans_refuse() {
    let raw = compact_legacy_rows().encode_data().unwrap();
    let mut changed = raw.to_vec();
    changed[4 + TAG.len() + 1] = 2;
    assert!(IndexData::decode_data(&changed, capability()).is_err());
    for tag in [5, 127, 255] {
        let mut changed = raw.to_vec();
        changed[header_length()] = tag;
        assert!(IndexData::decode_data(&changed, capability()).is_err());
    }
    let mut changed = raw.to_vec();
    // First compact row: tag,u64,framed identity,empty framed name,boolean.
    changed[header_length() + 1 + 8 + 36 + 4] = 2;
    assert!(IndexData::decode_data(&changed, capability()).is_err());
}

#[test]
fn native_data_context_and_active_source_labels_must_match() {
    let value = mixed();
    let raw = value.encode_data().unwrap();
    let other_layout = CurrentLayoutData {
        vault: [99; 32],
        root: [2; 32],
    };
    let other =
        NativeCapabilityData::for_current_data(other_layout, &other_layout.encode_data().unwrap())
            .unwrap();
    assert!(IndexData::decode_data(&raw, other).is_err());
    let mut changed = mixed();
    let BackendData::Founder(binding) = &mut changed.rows[2].backend else {
        panic!("fixture")
    };
    binding.vault = [99; 32];
    assert!(changed.encode_data().is_err());
    let mut changed = mixed();
    let BackendData::RetiredFounder { binding, .. } = &mut changed.rows[3].backend else {
        panic!("fixture")
    };
    binding.layout = [99; 32];
    assert!(changed.encode_data().is_err());
}

#[test]
fn native_data_generation_and_parent_geometry_is_exact() {
    let mut value = empty();
    value.generation = 0;
    assert!(value.encode_data().is_err());
    value.generation = 1;
    value.parent = Some([1; 32]);
    assert!(value.encode_data().is_err());
    value.generation = 2;
    value.parent = None;
    assert!(value.encode_data().is_err());
    for generation in [2, u64::MAX] {
        value.generation = generation;
        value.parent = Some([42; 32]);
        assert_eq!(
            IndexData::decode_data(&value.encode_data().unwrap(), capability()).unwrap(),
            value
        );
    }
}

#[test]
fn native_data_duplicate_or_reordered_numeric_ids_and_stable_ids_refuse() {
    let mut value = compact_legacy_rows();
    value.rows[1].id = value.rows[0].id;
    assert!(value.encode_data().is_err());
    let mut value = compact_legacy_rows();
    value.rows.swap(0, 1);
    assert!(value.encode_data().is_err());
    let mut value = compact_legacy_rows();
    value.rows[1].identity = value.rows[0].identity;
    assert!(value.encode_data().is_err());
    let raw = compact_legacy_rows().encode_data().unwrap();
    let row_bytes = 1 + 8 + 36 + 4 + 1 + 4;
    let mut duplicate_id = raw.to_vec();
    duplicate_id[header_length() + row_bytes + 1..header_length() + row_bytes + 9]
        .copy_from_slice(&0u64.to_be_bytes());
    assert!(IndexData::decode_data(&duplicate_id, capability()).is_err());
    let mut duplicate_identity = raw.to_vec();
    duplicate_identity
        [header_length() + row_bytes + 1 + 8 + 4..header_length() + row_bytes + 1 + 8 + 36]
        .fill(20);
    assert!(IndexData::decode_data(&duplicate_identity, capability()).is_err());
}

#[test]
fn native_data_count_and_total_frame_caps_refuse_before_count_driven_growth() {
    let mut raw = empty().encode_data().unwrap().to_vec();
    let end = raw.len();
    raw[end - 4..].copy_from_slice(&(MAX_ROWS as u32 + 1).to_be_bytes());
    assert!(IndexData::decode_data(&raw, capability()).is_err());
    assert!(IndexData::decode_data(&vec![0; MAX_PLAIN + 1], capability()).is_err());
    let mut value = empty();
    value.rows = (0..MAX_ROWS + 1)
        .map(|id| RowData {
            id: id as u64,
            identity: *blake3::hash(&(id as u64).to_be_bytes()).as_bytes(),
            name: String::new(),
            is_dm: false,
            backend: BackendData::Legacy {
                invite: String::new(),
            },
        })
        .collect();
    assert!(value.encode_data().is_err());
}

#[test]
fn native_data_individual_and_aggregate_string_caps_are_enforced() {
    let mut value = compact_legacy_rows();
    value.rows[0].name = "a".repeat(MAX_NAME + 1);
    assert!(value.encode_data().is_err());
    value.rows[0].name = "a".repeat(MAX_NAME);
    value.rows[0].backend = BackendData::Legacy {
        invite: "b".repeat(MAX_INVITE),
    };
    let encoded = value.encode_data().unwrap();
    assert_eq!(
        IndexData::decode_data(&encoded, capability()).unwrap(),
        value
    );
    value.rows[0].backend = BackendData::Legacy {
        invite: "b".repeat(MAX_INVITE + 1),
    };
    assert!(value.encode_data().is_err());
    let mut value = empty();
    value.rows = (0..64)
        .map(|id| RowData {
            id,
            identity: [id as u8; 32],
            name: String::new(),
            is_dm: false,
            backend: BackendData::Legacy {
                invite: "a".repeat(MAX_INVITE),
            },
        })
        .collect();
    assert!(
        value.encode_data().is_err(),
        "framing counts as well as string payload"
    );
}

#[test]
fn native_data_active_group_has_nonempty_exact_bound() {
    let mut value = mixed();
    let BackendData::Founder(binding) = &mut value.rows[2].backend else {
        panic!("fixture")
    };
    binding.group.clear();
    assert!(value.encode_data().is_err());
    let mut value = mixed();
    let BackendData::Founder(binding) = &mut value.rows[2].backend else {
        panic!("fixture")
    };
    binding.group = vec![1; MAX_GROUP + 1];
    assert!(value.encode_data().is_err());
    let mut value = mixed();
    let BackendData::Founder(binding) = &mut value.rows[2].backend else {
        panic!("fixture")
    };
    binding.group = vec![1; MAX_GROUP];
    assert_eq!(
        IndexData::decode_data(&value.encode_data().unwrap(), capability()).unwrap(),
        value
    );
}

#[test]
fn native_data_legacy_migration_is_deterministic_complete_and_canonical() {
    let records = vec![
        ServerRecord {
            id: 5,
            display_name: "later".into(),
            invite: "saved invite".into(),
            is_dm: false,
        },
        ServerRecord {
            id: 0,
            display_name: "first".into(),
            invite: String::new(),
            is_dm: true,
        },
    ];
    let migrated = IndexData::migrate_legacy_data(capability(), Some([55; 32]), &records).unwrap();
    assert_eq!(
        migrated.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        [0, 5]
    );
    assert_eq!(migrated.rows[0].name, "first");
    assert!(migrated.rows[0].is_dm);
    assert!(
        matches!(&migrated.rows[1].backend, BackendData::Legacy { invite } if invite == "saved invite")
    );
    assert_eq!(
        migrated,
        IndexData::migrate_legacy_data(capability(), Some([55; 32]), &records).unwrap()
    );
    let other_parent =
        IndexData::migrate_legacy_data(capability(), Some([56; 32]), &records).unwrap();
    assert_ne!(migrated.rows[0].identity, other_parent.rows[0].identity);
    assert_eq!(
        IndexData::decode_data(&migrated.encode_data().unwrap(), capability()).unwrap(),
        migrated
    );
    let mut duplicate = records.clone();
    duplicate.push(records[0].clone());
    assert!(IndexData::migrate_legacy_data(capability(), Some([55; 32]), &duplicate).is_err());
}

#[test]
fn native_data_pending_and_abandoned_legacy_intents_stay_distinct_from_active_rows() {
    let mut value = mixed();
    let BackendData::Pending(pending) = &mut value.rows[1].backend else {
        panic!("fixture")
    };
    pending.backend = CreationBackendData::Legacy;
    let BackendData::AbandonedCreate {
        pending, reason, ..
    } = &mut value.rows[4].backend
    else {
        panic!("fixture")
    };
    pending.backend = CreationBackendData::Legacy;
    *reason = AbandonedCreateReasonData::BeforeSdk;
    assert_eq!(
        IndexData::decode_data(&value.encode_data().unwrap(), capability()).unwrap(),
        value
    );
    assert!(!format!("{value:?}").contains("grammar-only"));
}
