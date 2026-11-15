//! JOIN re-encoders over catalogs that carry namespace5, with and without governance4.
//! DATA only: rows are synthetic original Pending source bytes, never a real source, grant
//! or submission; no catalog is written and no handler is mounted.
use super::*;
use crate::store::critical_namespaces::{
    archive_bootstrap, archive_current_namespace_codec as current, catalog_envelope as outer,
    governance_namespace_codec, member_outbox_namespace_codec as member,
};

const VAULT: [u8; 32] = [0x71; 32];
const ROOT: [u8; 32] = [0x72; 32];
type Entries = Vec<(u32, u32, Vec<u8>)>;

fn encoded(generation: u64, parent: &[u8; 32], entries: &Entries) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(outer::TAG_V2).unwrap();
    e.put_bytes(&VAULT).unwrap();
    e.put_u64(generation);
    e.put_bytes(parent).unwrap();
    e.put_u32(entries.len() as u32);
    for (id, version, payload) in entries {
        e.put_u32(*id).put_u32(*version);
        e.put_bytes(payload).unwrap();
    }
    e.finish()
}

fn entries(raw: &[u8]) -> Entries {
    let parsed = outer::parse(raw, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
    (0..parsed.count)
        .map(|index| {
            let e = parsed.entry(index).unwrap();
            (e.id, e.version, e.payload.to_vec())
        })
        .collect()
}

/// One framed original Pending row; `n` keeps source, group, signer and nonce distinct.
fn row(n: u8, revision: u64, paused: bool) -> Vec<u8> {
    let mut immutable = Encoder::new();
    immutable.put_str(codec::IMMUTABLE_TAG).unwrap();
    immutable.put_bytes(&VAULT).unwrap();
    immutable.put_bytes(&[n; 32]).unwrap();
    immutable.put_u8(1).put_u8(1);
    let group = [b'g', n];
    for field in [
        &group[..],
        &[n.wrapping_add(1); 32],
        &[0x73; 32],
        &[0x74; 32],
        &[n.wrapping_add(2); 16],
        &[0x75; 32],
        &[0x76; 32],
        &[0x77; 32],
    ] {
        immutable.put_bytes(field).unwrap();
    }
    immutable.put_bytes(&[0]).unwrap();
    let mut row = Encoder::new();
    row.put_bytes(&[n; 32]).unwrap();
    row.put_u64(revision).put_u8(u8::from(paused)).put_u8(1);
    row.put_bytes(&immutable.finish()).unwrap();
    row.put_bytes(&[0x78; 40]).unwrap();
    row.finish()
}

fn join(rows: &[Vec<u8>]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(codec::JOIN_TAG).unwrap();
    e.put_u32(rows.len() as u32);
    for row in rows {
        e.put_bytes(row).unwrap();
    }
    e.finish()
}

/// [JOIN(one paused row), 2, 3] then the requested namespace declarations, in order.
fn catalog(declare: &[u32]) -> Zeroizing<Vec<u8>> {
    let values = vec![
        (
            codec::JOIN_ID,
            codec::JOIN_VERSION,
            join(&[row(0x10, 3, true)]),
        ),
        (
            archive_bootstrap::ARCHIVE_ID,
            archive_bootstrap::ARCHIVE_VERSION,
            archive_bootstrap::Descriptor {
                vault: VAULT,
                root: ROOT,
            }
            .encode()
            .unwrap()
            .to_vec(),
        ),
        (
            current::NAMESPACE_ID,
            current::NAMESPACE_VERSION,
            current::CurrentLayoutData {
                vault: VAULT,
                root: ROOT,
            }
            .encode_data()
            .unwrap()
            .to_vec(),
        ),
    ];
    let mut raw = Zeroizing::new(encoded(9, &[0x79; 32], &values));
    for id in declare {
        raw = match id {
            4 => governance_namespace_codec::prepare_declaration_data(&raw, &VAULT).unwrap(),
            5 => member::prepare_declaration_data(&raw, &VAULT).unwrap(),
            _ => unreachable!(),
        };
    }
    raw
}

/// Every non-JOIN entry of `next` equals `current`'s, byte for byte and in order.
fn assert_namespaces_kept(current: &[u8], next: &[u8]) {
    let keep = |raw: &[u8]| {
        entries(raw)
            .into_iter()
            .filter(|e| e.0 != codec::JOIN_ID)
            .collect::<Vec<_>>()
    };
    assert_eq!(keep(current), keep(next));
}

#[test]
fn schedule_toggle_and_add_preserve_five_with_and_without_four() {
    for declare in [&[5][..], &[5, 4][..], &[4, 5][..]] {
        let raw = catalog(declare);
        let parsed = codec::parse_all_known_for_test(&raw, &VAULT, false).unwrap();
        assert!(parsed.member_outbox.is_some());
        assert_eq!(parsed.governance.is_some(), declare.contains(&4));
        let summary = parsed.summary([0; 32]);

        // Pause/resume toggle of the existing row.
        let mut target = parsed.rows[0].as_ref().unwrap().metadata.clone();
        target.revision += 1;
        target.scheduling = codec::Scheduling::Active;
        let toggled = encode_schedule(&parsed, &target, &VAULT).unwrap();
        assert_namespaces_kept(&raw, &toggled);
        let next = codec::parse_all_known_for_test(&toggled, &VAULT, false).unwrap();
        assert_eq!(next.member_outbox, parsed.member_outbox);
        assert_eq!(next.member_outbox_payload, parsed.member_outbox_payload);
        assert_eq!(next.governance_payload, parsed.governance_payload);
        codec::successor(&summary, &next.summary([1; 32])).unwrap();

        // Adding a new original Pending row.
        let added_bytes = row(0x20, 1, false);
        let added = codec::parse_original_row(&added_bytes, &VAULT).unwrap();
        let grown = encode_added(
            &parsed,
            &added.metadata,
            added.immutable,
            added.encrypted,
            &VAULT,
        )
        .unwrap();
        assert_namespaces_kept(&raw, &grown);
        let next = codec::parse_all_known_for_test(&grown, &VAULT, false).unwrap();
        assert_eq!(next.member_outbox_payload, parsed.member_outbox_payload);
        codec::successor(&summary, &next.summary([2; 32])).unwrap();

        // An opener with only the member handler reads the 5-only catalogs; any catalog with
        // 4 also needs the governance handler, and no opener without the member handler
        // reads any of them.
        assert_eq!(
            codec::parse_configured_current_layout(&toggled, &VAULT, false, true).is_ok(),
            !declare.contains(&4)
        );
        assert!(codec::parse_configured_current_layout(&toggled, &VAULT, false, false).is_err());
    }
}

#[test]
fn a_reencoding_that_loses_or_changes_five_is_not_a_successor() {
    let raw = catalog(&[5]);
    let parsed = codec::parse_all_known_for_test(&raw, &VAULT, false).unwrap();
    let summary = parsed.summary([0; 32]);
    let mut target = parsed.rows[0].as_ref().unwrap().metadata.clone();
    target.revision += 1;
    target.scheduling = codec::Scheduling::Active;
    let toggled = encode_schedule(&parsed, &target, &VAULT).unwrap();
    let parent = outer::catalog_digest(&raw);

    let mut lost = entries(&toggled);
    lost.retain(|e| e.0 != member::NAMESPACE_ID);
    let lost = encoded(11, &parent, &lost);
    let lost = codec::parse_all_known_for_test(&lost, &VAULT, false).unwrap();
    assert!(codec::successor(&summary, &lost.summary([1; 32])).is_err());

    // Same length, same descriptor shape, different bytes: a re-created descriptor for a
    // different ID3 commitment cannot pass as the preserved one.
    let mut changed = entries(&toggled);
    let five = changed
        .iter_mut()
        .find(|e| e.0 == member::NAMESPACE_ID)
        .unwrap();
    let last = five.2.len() - 1;
    five.2[last] ^= 1;
    let changed = encoded(11, &parent, &changed);
    assert!(codec::parse_all_known_for_test(&changed, &VAULT, false).is_err());
}

/// Codec-only evidence for both declaration orders. The true native opener still requires
/// its actual physical handlers; this all-known grammar manufactures no mounted capability.
fn with_native(declare_four_first: Option<bool>) -> Zeroizing<Vec<u8>> {
    use super::super::super::{
        catalog_declaration, native_registry::capability::NativeCapabilityData,
    };
    let base = catalog(if declare_four_first == Some(true) {
        &[4, 5]
    } else {
        &[5]
    });
    let layout = current::CurrentLayoutData {
        vault: VAULT,
        root: ROOT,
    };
    let cap =
        NativeCapabilityData::for_current_data(layout, &layout.encode_data().unwrap()).unwrap();
    let mut next =
        catalog_declaration::insert_entry_data(&base, &VAULT, 7, 1, &cap.encode_data().unwrap())
            .unwrap();
    if declare_four_first == Some(false) {
        let four = entries(&catalog(&[4, 5]))
            .into_iter()
            .find(|entry| entry.0 == 4)
            .unwrap();
        next = catalog_declaration::insert_entry_data(&next, &VAULT, 4, four.1, &four.2).unwrap();
    }
    next
}

#[test]
fn native_registry_join_add_pause_resume_preserve_seven_in_both_declaration_orders() {
    for four in [None, Some(false), Some(true)] {
        let original = with_native(four);
        let parsed =
            codec::parse_native_registry_layout(&original, &VAULT, false, four.is_some()).unwrap();
        let summary = parsed.summary([0; 32]);
        assert!(parsed.native_registry_payload.is_some());
        assert!(codec::parse_configured_current_layout(&original, &VAULT, false, true).is_err());
        let mut target = parsed.rows[0].as_ref().unwrap().metadata.clone();
        target.revision += 1;
        target.scheduling = codec::Scheduling::Active;
        let resumed = encode_schedule(&parsed, &target, &VAULT).unwrap();
        assert_namespaces_kept(&original, &resumed);
        let next =
            codec::parse_native_registry_layout(&resumed, &VAULT, false, four.is_some()).unwrap();
        codec::successor(&summary, &next.summary([1; 32])).unwrap();
        target.revision += 1;
        target.scheduling = codec::Scheduling::Paused;
        let paused = encode_schedule(&next, &target, &VAULT).unwrap();
        assert_namespaces_kept(&original, &paused);
        let last =
            codec::parse_native_registry_layout(&paused, &VAULT, false, four.is_some()).unwrap();
        codec::successor(&next.summary([1; 32]), &last.summary([2; 32])).unwrap();
        let added_bytes = row(0x20, 1, false);
        let added = codec::parse_original_row(&added_bytes, &VAULT).unwrap();
        let grown = encode_added(
            &last,
            &added.metadata,
            added.immutable,
            added.encrypted,
            &VAULT,
        )
        .unwrap();
        assert_namespaces_kept(&original, &grown);
        let grown =
            codec::parse_native_registry_layout(&grown, &VAULT, false, four.is_some()).unwrap();
        codec::successor(&last.summary([2; 32]), &grown.summary([3; 32])).unwrap();
    }
}

#[test]
fn native_registry_join_successor_refuses_lost_changed_or_unknown_seven() {
    let original = with_native(None);
    let parsed = codec::parse_native_registry_layout(&original, &VAULT, false, false).unwrap();
    let mut target = parsed.rows[0].as_ref().unwrap().metadata.clone();
    target.revision += 1;
    target.scheduling = codec::Scheduling::Active;
    let resumed = encode_schedule(&parsed, &target, &VAULT).unwrap();
    let mut lost = entries(&resumed);
    lost.retain(|entry| entry.0 != 7);
    let lost = encoded(parsed.generation + 1, &parsed.digest, &lost);
    let lost = codec::parse_native_registry_layout(&lost, &VAULT, false, false).unwrap();
    assert!(codec::successor(&parsed.summary([0; 32]), &lost.summary([1; 32])).is_err());
    for unknown_version in [false, true] {
        let mut changed = entries(&resumed);
        let seven = changed.iter_mut().find(|entry| entry.0 == 7).unwrap();
        if unknown_version {
            seven.1 = 2;
        } else {
            let last = seven.2.len() - 1;
            seven.2[last] ^= 1;
        }
        let changed = encoded(parsed.generation + 1, &parsed.digest, &changed);
        assert!(codec::parse_native_registry_layout(&changed, &VAULT, false, false).is_err());
    }
}
