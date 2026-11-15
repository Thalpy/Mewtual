//! Pure syntax fixtures, not authenticated sources or execution authorities.
//! Real held-file/JOIN lifecycle tests belong to the physical upgrade.
use super::*;
use crate::store::critical_namespaces::{
    archive_bootstrap,
    archive_current_namespace_codec::CurrentLayoutData,
    catalog_declaration,
    pending_join::{execution_payload, execution_state},
};

const VAULT: [u8; 32] = [0x73; 32];

fn original(join_version: Option<u32>, paused: bool) -> (Vec<u8>, NativeCapabilityData) {
    let layout = CurrentLayoutData {
        vault: VAULT,
        root: [0x74; 32],
    };
    let current = layout.encode_data().unwrap();
    let cap = NativeCapabilityData::for_current_data(layout, &current).unwrap();
    let mut out = Encoder::new();
    out.put_str(outer::TAG_V2).unwrap();
    out.put_bytes(&VAULT).unwrap();
    out.put_u64(9);
    out.put_bytes(&[0x75; 32]).unwrap();
    out.put_u32(2 + u32::from(join_version.is_some()));
    if let Some(version) = join_version {
        out.put_u32(1).put_u32(version);
        out.put_bytes(&join(version, paused)).unwrap();
    }
    let archive = archive_bootstrap::Descriptor {
        vault: VAULT,
        root: layout.root,
    }
    .encode()
    .unwrap();
    for (id, bytes) in [(2, archive.as_slice()), (3, current.as_slice())] {
        out.put_u32(id).put_u32(1);
        out.put_bytes(bytes).unwrap();
    }
    let member =
        crate::store::critical_namespaces::member_outbox_namespace_codec::prepare_declaration_data(
            &out.finish(),
            &VAULT,
        )
        .unwrap();
    let native = catalog_declaration::insert_entry_data(
        &member,
        &VAULT,
        7,
        2,
        &v2::Capability { original: cap }.encode_data().unwrap(),
    )
    .unwrap();
    (native.to_vec(), cap)
}

fn join(version: u32, paused: bool) -> Vec<u8> {
    let mut immutable = Encoder::new();
    immutable.put_str(grammar::IMMUTABLE_TAG).unwrap();
    immutable.put_bytes(&VAULT).unwrap();
    immutable.put_bytes(&[0x76; 32]).unwrap();
    immutable.put_u8(1).put_u8(1);
    for bytes in [
        &b"retained join"[..],
        &[0x77; 32],
        &[0x78; 32],
        &[0x79; 32],
        &[0x7a; 16],
        &[0x7b; 32],
        &[0x7c; 32],
        &[0x7d; 32],
    ] {
        immutable.put_bytes(bytes).unwrap();
    }
    immutable.put_bytes(&[0]).unwrap();
    let immutable = immutable.finish();
    let identity = execution_state::SourceIdentity {
        binding: grammar::binding_digest(&immutable),
        request: [0x7c; 32],
        package: [0x7d; 32],
    };
    let mut row = Encoder::new();
    row.put_bytes(&[0x76; 32]).unwrap();
    row.put_u64(3).put_u8(u8::from(paused)).put_u8(1);
    row.put_bytes(&immutable).unwrap();
    row.put_bytes(&[0x7e; 40]).unwrap();
    let row = if version == 2 {
        // A spent attempt is history too. The migration must not reset it to
        // NotGranted or infer that a paused source was never submitted.
        let state = execution_state::State {
            grant: 1,
            limit: 2,
            spent: 1,
            ordinal: 1,
            arm: Some(execution_state::Arm {
                source: identity,
                grant: 1,
                ordinal: 1,
                scheduling_revision: 2,
                activation: [0x7f; 32],
                invocation: [0x80; 32],
                deadline_ms: 1000,
            }),
            input: None,
        };
        let mut execution = Encoder::new();
        execution_state::encode_into(&state, None, &identity, 3, &mut execution).unwrap();
        let mut nested = Encoder::new();
        nested.put_bytes(&row.finish()).unwrap();
        nested.put_bytes(&execution.finish()).unwrap();
        nested.finish()
    } else {
        row.finish()
    };
    let mut payload = Encoder::new();
    payload
        .put_str(if version == 2 {
            execution_payload::TAG
        } else {
            grammar::JOIN_TAG
        })
        .unwrap();
    payload.put_u32(1);
    payload.put_bytes(&row).unwrap();
    payload.finish()
}

fn assert_pair(before: &[u8], after: &[u8], cap: NativeCapabilityData) {
    let a = outer::parse(before, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    let b = outer::parse(after, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    assert_eq!(b.generation, a.generation + 1);
    assert_eq!(b.parent, Some(a.digest));
    assert_eq!(b.count, a.count);
    for index in 0..a.count {
        let old = a.entry(index).unwrap();
        let new = b.entry(index).unwrap();
        assert_eq!(new.id, old.id);
        if old.id == 7 {
            assert_eq!((old.version, new.version), (2, 3));
            Capability::decode_exact(new.payload, Capability { original: cap }).unwrap();
        } else {
            assert_eq!(new.version, old.version);
            assert_eq!(new.payload, old.payload);
        }
    }
    assert!(grammar::parse_native_registry_layout(after, &VAULT, true, false).is_err());
    assert!(grammar::parse_operational_native_registry_layout(after, &VAULT, true).is_err());
    assert!(grammar::parse_lifecycle_native_registry_layout(before, &VAULT, true).is_err());
    grammar::parse_lifecycle_native_registry_layout(after, &VAULT, true).unwrap();
}

#[test]
fn lifecycle3_catalog_upgrade_preserves_all_active_paused_and_spent_join_bytes() {
    for version in [None, Some(1), Some(2)] {
        for paused in [false, true] {
            let (before, cap) = original(version, paused);
            let after = upgrade_data(&before, &VAULT, cap).unwrap();
            assert_pair(&before, &after, cap);
            let parsed =
                grammar::parse_lifecycle_native_registry_layout(&after, &VAULT, true).unwrap();
            assert_eq!(parsed.count, usize::from(version.is_some()));
            assert_eq!(parsed.join_version, version);
            if let Some(version) = version {
                let row = &parsed.rows[0].as_ref().unwrap().metadata;
                assert_eq!(row.scheduling == grammar::Scheduling::Paused, paused);
                assert_eq!(row.revision, 3);
                if version == 2 {
                    assert_eq!(row.execution.unwrap().spent, 1);
                    assert!(
                        grammar::parse_lifecycle_native_registry_layout(&after, &VAULT, false)
                            .is_err()
                    );
                }
            }
        }
    }
}

#[test]
fn lifecycle3_catalog_upgrade_refuses_foreign_context_repeat_and_generation_exhaustion() {
    let (before, cap) = original(Some(2), true);
    let after = upgrade_data(&before, &VAULT, cap).unwrap();
    assert!(upgrade_data(&after, &VAULT, cap).is_err());
    assert!(upgrade_data(&before, &[0x99; 32], cap).is_err());
    let layout = CurrentLayoutData {
        vault: VAULT,
        root: [0x99; 32],
    };
    let foreign =
        NativeCapabilityData::for_current_data(layout, &layout.encode_data().unwrap()).unwrap();
    assert!(upgrade_data(&before, &VAULT, foreign).is_err());
    let mut exhausted = before;
    let at = 4 + outer::TAG_V2.len() + 36;
    exhausted[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(upgrade_data(&exhausted, &VAULT, cap).is_err());
}

#[test]
fn lifecycle3_catalog_upgrade_refuses_unknown_entries_and_every_truncated_original() {
    let (before, cap) = original(None, false);
    let future = catalog_declaration::insert_entry_data(&before, &VAULT, 9, 1, b"unknown").unwrap();
    assert!(upgrade_data(&future, &VAULT, cap).is_err());
    for length in 0..before.len() {
        assert!(
            upgrade_data(&before[..length], &VAULT, cap).is_err(),
            "cut {length}"
        );
    }
    let mut trailing = before;
    trailing.push(0);
    assert!(upgrade_data(&trailing, &VAULT, cap).is_err());
}
