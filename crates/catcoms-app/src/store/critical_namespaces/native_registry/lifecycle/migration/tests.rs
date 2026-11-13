//! Real predecessor byte codec -> exact owned DATA conversion. These fixtures
//! are not physical authenticated upgrades, SDK products or native credentials.
use super::*;
use crate::store::critical_namespaces::{
    archive_current_namespace_codec::CurrentLayoutData,
    native_registry::{
        data as original,
        operational::{capability as old_cap, data as old},
    },
};
use zeroize::Zeroizing;

const OLD_FRAME: [u8; 32] = [80; 32];
const PARENT: [u8; 32] = [81; 32];
fn cap() -> old_cap::Capability {
    let layout = CurrentLayoutData {
        vault: [1; 32],
        root: [2; 32],
    };
    old_cap::Capability::for_current_data(layout, &layout.encode_data().unwrap()).unwrap()
}
fn identity(id: u64) -> [u8; 32] {
    let mut value = [0; 32];
    value[..8].copy_from_slice(&id.to_be_bytes());
    value
}
fn old_birth(id: u64) -> old::Birth {
    old::Birth {
        invocation: identity(id + 1000),
        request: identity(id + 2000),
        signer: [3; 32],
        network: identity(id + 3000),
    }
}
fn old_binding(capability: old_cap::Capability) -> original::FounderBindingData {
    original::FounderBindingData {
        vault: capability.original.vault,
        layout: capability.original.root,
        group: vec![4; 256],
        signer: [3; 32],
        source: [5; 32],
        profile: [6; 32],
        pin: [7; 32],
    }
}
fn decode_original(value: &old::Index) -> old::Index {
    let raw = value.encode_data().unwrap();
    old::Index::decode_data(&raw, value.capability).unwrap()
}
fn candidate(value: &Index) -> Index {
    let mut next = Index::decode_data(&value.encode_data().unwrap(), value.capability).unwrap();
    next.generation += 1;
    next.parent = Some(PARENT);
    next
}

#[test]
fn lifecycle3_full_size_predecessor_legacy_table_can_rename_and_retire_after_conversion() {
    let mut source = old::Index {
        capability: cap(),
        generation: 1,
        parent: None,
        last_issued_id: 64,
        rows: (1..=64)
            .map(|id| old::Row {
                id,
                identity: identity(id),
                name: String::new(),
                is_dm: id % 2 == 0,
                backend: old::Backend::Legacy {
                    invite: String::new(),
                },
            })
            .collect(),
    };
    let mut remaining = old::MAX_PLAIN - source.encoded_length().unwrap();
    for row in &mut source.rows {
        let old::Backend::Legacy { invite } = &mut row.backend else {
            unreachable!()
        };
        let bytes = remaining.min(original::MAX_INVITE);
        *invite = "i".repeat(bytes);
        remaining -= bytes;
    }
    assert_eq!(remaining, 0);
    assert_eq!(source.encode_data().unwrap().len(), old::MAX_PLAIN);
    let mut converted = Index::from_v2_data(decode_original(&source), OLD_FRAME).unwrap();
    assert_eq!(converted.origin.generation, 1);
    assert_eq!(converted.origin.frame, OLD_FRAME);
    assert_eq!(converted.last_issued_id, 64);
    let mut renamed = candidate(&converted);
    renamed.rows[0].name = "x".into();
    renamed.rows[0].metadata_revision = 1;
    assert_eq!(
        converted.successor_data(&renamed, PARENT).unwrap(),
        Change::Rename
    );
    renamed.encode_data().unwrap();
    converted = renamed;
    // Complete every imported row, not only the first small invitation.
    for index in 0..converted.rows.len() {
        let before = converted.reserved_length_data().unwrap();
        let mut retired = candidate(&converted);
        let Backend::Legacy { origin, invite } = &converted.rows[index].backend else {
            unreachable!()
        };
        retired.rows[index].backend = Backend::RetiredLegacy {
            origin: origin.clone(),
            invite: invite.clone(),
            terminal: identity(index as u64 + 4000),
        };
        assert_eq!(
            converted.successor_data(&retired, PARENT).unwrap(),
            Change::Retire
        );
        assert_eq!(retired.reserved_length_data().unwrap(), before);
        retired.encode_data().unwrap();
        converted = retired;
    }
    assert_eq!(converted.rows.len(), source.rows.len());
    for (old, retired) in source.rows.iter().zip(&converted.rows) {
        assert_eq!(
            (old.id, old.identity, old.is_dm),
            (retired.id, retired.identity, retired.is_dm)
        );
        let (
            old::Backend::Legacy { invite: a },
            Backend::RetiredLegacy {
                origin: LegacyOrigin::Imported,
                invite: b,
                ..
            },
        ) = (&old.backend, &retired.backend)
        else {
            unreachable!()
        };
        assert_eq!(a, b);
    }
}

#[test]
fn lifecycle3_full_predecessor_pending_table_keeps_minimum_frame_activation_and_terminal_capacity()
{
    let mut source = old::Index {
        capability: cap(),
        generation: 66,
        parent: Some(OLD_FRAME),
        last_issued_id: 65,
        rows: (1..=65)
            .map(|id| old::Row {
                id,
                identity: identity(id),
                name: String::new(),
                is_dm: false,
                backend: old::Backend::Pending {
                    birth: old_birth(id),
                    network_frame: Zeroizing::new(vec![8; 40]),
                },
            })
            .collect(),
    };
    let mut remaining = old::MAX_PLAIN - source.encoded_length().unwrap();
    for row in source.rows.iter_mut().skip(1) {
        let old::Backend::Pending { network_frame, .. } = &mut row.backend else {
            unreachable!()
        };
        let growth = remaining.min(old::MAX_NETWORK_FRAME - 40);
        network_frame.resize(40 + growth, 8);
        remaining -= growth;
    }
    assert_eq!(remaining, 0);
    assert_eq!(source.encode_data().unwrap().len(), old::MAX_PLAIN);
    let original = decode_original(&source);
    let old::Backend::Pending { network_frame, .. } = &original.rows[0].backend else {
        unreachable!()
    };
    let original_pointer = network_frame.as_ptr();
    let converted = Index::from_v2_data(original, OLD_FRAME).unwrap();
    let Backend::Pending {
        birth,
        network_frame,
    } = &converted.rows[0].backend
    else {
        unreachable!()
    };
    assert_eq!(
        network_frame.as_ptr(),
        original_pointer,
        "conversion moves the same encrypted Vec"
    );
    assert_eq!(network_frame.len(), 40);
    let mut active = candidate(&converted);
    active.rows[0].backend = Backend::Founder {
        birth: birth.clone(),
        binding: old_binding(source.capability),
    };
    assert_eq!(
        converted.successor_data(&active, PARENT).unwrap(),
        Change::Activate
    );
    assert_eq!(
        active.reserved_length_data().unwrap(),
        converted.reserved_length_data().unwrap()
    );
    active.encode_data().unwrap();
    let mut retired = candidate(&active);
    let Backend::Founder { birth, binding } = &active.rows[0].backend else {
        unreachable!()
    };
    retired.rows[0].backend = Backend::RetiredFounder {
        birth: birth.clone(),
        binding: binding.clone(),
        terminal: [9; 32],
    };
    assert_eq!(
        active.successor_data(&retired, PARENT).unwrap(),
        Change::Retire
    );
    assert_eq!(
        active.reserved_length_data().unwrap(),
        retired.reserved_length_data().unwrap()
    );
    retired.encode_data().unwrap();
    for (old, new) in source.rows.iter().zip(&converted.rows) {
        let (
            old::Backend::Pending {
                birth: a,
                network_frame: x,
            },
            Backend::Pending {
                birth: b,
                network_frame: y,
            },
        ) = (&old.backend, &new.backend)
        else {
            unreachable!()
        };
        assert_eq!(a.invocation, b.invocation);
        assert_eq!(a.request, b.request);
        assert_eq!(a.signer, b.signer);
        assert_eq!(a.network, b.network);
        assert_eq!(x.as_slice(), y.as_slice());
    }
}

#[test]
fn lifecycle3_conversion_keeps_all_terminal_receipts_and_the_original_highwater() {
    let capability = cap();
    let rows = vec![
        old::Row {
            id: 1,
            identity: identity(1),
            name: "imported".into(),
            is_dm: true,
            backend: old::Backend::Legacy {
                invite: "old".into(),
            },
        },
        old::Row {
            id: 2,
            identity: identity(2),
            name: "pending".into(),
            is_dm: false,
            backend: old::Backend::Pending {
                birth: old_birth(2),
                network_frame: Zeroizing::new(vec![2; 40]),
            },
        },
        old::Row {
            id: 3,
            identity: identity(3),
            name: "founder".into(),
            is_dm: false,
            backend: old::Backend::Founder {
                birth: old_birth(3),
                binding: old_binding(capability),
            },
        },
        old::Row {
            id: 4,
            identity: identity(4),
            name: "retired".into(),
            is_dm: false,
            backend: old::Backend::Retired {
                birth: old_birth(4),
                binding: old_binding(capability),
                scope_terminal: [10; 32],
            },
        },
        old::Row {
            id: 5,
            identity: identity(5),
            name: "abandoned".into(),
            is_dm: false,
            backend: old::Backend::Abandoned {
                birth: old_birth(5),
                reason: original::AbandonedCreateReasonData::BeforeSdk,
                journal_terminal: [11; 32],
            },
        },
        old::Row {
            id: 6,
            identity: identity(6),
            name: "abandoned after SDK".into(),
            is_dm: true,
            backend: old::Backend::Abandoned {
                birth: old_birth(6),
                reason: original::AbandonedCreateReasonData::UnexposedAfterSdk,
                journal_terminal: [12; 32],
            },
        },
    ];
    let source = old::Index {
        capability,
        generation: 99,
        parent: Some(OLD_FRAME),
        last_issued_id: 123,
        rows,
    };
    let converted = Index::from_v2_data(decode_original(&source), OLD_FRAME).unwrap();
    assert_eq!(converted.last_issued_id, 123);
    assert_eq!(converted.origin.generation, 99);
    assert!(matches!(
        converted.rows[0].backend,
        Backend::Legacy {
            origin: LegacyOrigin::Imported,
            ..
        }
    ));
    for (old, new) in source.rows.iter().zip(&converted.rows) {
        assert_eq!(
            (old.id, old.identity, old.is_dm),
            (new.id, new.identity, new.is_dm)
        );
        assert_eq!(old.name, new.name);
        assert_eq!(new.metadata_revision, 0);
    }
    for id in 2..=6 {
        let old = old_birth(id);
        let row = converted
            .lookup_data(old.invocation, old.request)
            .unwrap()
            .unwrap();
        assert_eq!(row.id, id);
        let birth = row.backend.birth().unwrap();
        assert!(birth.kind == BirthKind::Founder);
        assert_eq!(birth.join_context, None);
        assert_eq!((birth.signer, birth.network), (old.signer, old.network));
    }
    let Backend::Founder { binding, .. } = &converted.rows[2].backend else {
        unreachable!()
    };
    assert!(*binding == old_binding(capability));
    let Backend::RetiredFounder {
        terminal, binding, ..
    } = &converted.rows[3].backend
    else {
        unreachable!()
    };
    assert_eq!(*terminal, [10; 32]);
    assert!(*binding == old_binding(capability));
    let Backend::AbandonedFounder {
        terminal, reason, ..
    } = &converted.rows[4].backend
    else {
        unreachable!()
    };
    assert_eq!(*terminal, [11; 32]);
    assert!(*reason == original::AbandonedCreateReasonData::BeforeSdk);
    let Backend::AbandonedFounder {
        terminal, reason, ..
    } = &converted.rows[5].backend
    else {
        unreachable!()
    };
    assert_eq!(*terminal, [12; 32]);
    assert!(*reason == original::AbandonedCreateReasonData::UnexposedAfterSdk);
}
