//! Catalog syntax/equality only. Nested JOIN ciphertext below is deliberately DATA,
//! not a source, admission, execution grant or SDK operation permit.

use super::*;
use crate::store::critical_namespaces::{archive_bootstrap, encode_empty};

const VAULT: [u8; 32] = [0x44; 32];
const ROOT: [u8; 32] = [0x45; 32];
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

fn join_payload(version: u32) -> Vec<u8> {
    let mut immutable = Encoder::new();
    immutable.put_str(prior::IMMUTABLE_TAG).unwrap();
    immutable.put_bytes(&VAULT).unwrap();
    immutable.put_bytes(&[0x46; 32]).unwrap();
    immutable.put_u8(1).put_u8(1);
    for field in [
        &b"data-group"[..],
        &[0x47; 32],
        &[0x48; 32],
        &[0x49; 32],
        &[0x4a; 16],
        &[0x4b; 32],
        &[0x4c; 32],
        &[0x4d; 32],
    ] {
        immutable.put_bytes(field).unwrap();
    }
    immutable.put_bytes(&[0]).unwrap();
    let mut row = Encoder::new();
    row.put_bytes(&[0x46; 32]).unwrap();
    row.put_u64(3).put_u8(1).put_u8(1); // paused original Pending DATA
    row.put_bytes(&immutable.finish()).unwrap();
    row.put_bytes(&[0x4e; 40]).unwrap();
    let original = row.finish();
    let mut e = Encoder::new();
    match version {
        1 => {
            e.put_str(prior::JOIN_TAG).unwrap();
            e.put_u32(1);
            e.put_bytes(&original).unwrap();
        }
        2 => {
            use crate::store::critical_namespaces::pending_join::{
                execution_payload as payload, execution_state,
            };
            let mut parsed = prior::parse_original_row(&original, &VAULT).unwrap();
            parsed.metadata.execution = Some(execution_state::State::not_granted());
            e.put_str(payload::TAG).unwrap();
            e.put_u32(1);
            payload::encode_row(
                &mut e,
                &parsed.metadata,
                parsed.immutable,
                parsed.encrypted,
                None,
            )
            .unwrap();
        }
        _ => unreachable!(),
    }
    e.finish()
}

fn original(join_version: Option<u32>, generation: u64) -> Vec<u8> {
    let mut values = Vec::new();
    if let Some(version) = join_version {
        values.push((prior::JOIN_ID, version, join_payload(version)));
    }
    values.push((
        archive_bootstrap::ARCHIVE_ID,
        archive_bootstrap::ARCHIVE_VERSION,
        archive_bootstrap::Descriptor {
            vault: VAULT,
            root: ROOT,
        }
        .encode()
        .unwrap()
        .to_vec(),
    ));
    values.push((
        current::NAMESPACE_ID,
        current::NAMESPACE_VERSION,
        current::CurrentLayoutData {
            vault: VAULT,
            root: ROOT,
        }
        .encode_data()
        .unwrap()
        .to_vec(),
    ));
    encoded(generation, &[0x4f; 32], &values)
}

#[test]
fn declaration_adds_only_four_and_preserves_all_original_join_versions_and_bytes() {
    for join in [None, Some(1), Some(2)] {
        let old = original(join, 17);
        let new = prepare_declaration_data(&old, &VAULT).unwrap();
        validate_declaration_pair_data(&old, &new, &VAULT).unwrap();
        let old_envelope = outer::parse(&old, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
        let next = outer::parse(&new, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
        assert_eq!(next.generation, 18);
        assert_eq!(next.parent, Some(outer::catalog_digest(&old)));
        assert_eq!(&entries(&new)[..old_envelope.count], entries(&old));
        let added = next.entry(old_envelope.count).unwrap();
        assert_eq!((added.id, added.version), (4, 1));
        assert_eq!(added.payload.len(), DESCRIPTOR_BYTES);
        assert!(prior::parse(&new, &VAULT).is_err());
        assert!(prior::parse_bootstrap(&new, &VAULT).is_err());
        assert!(prior::parse_current_layout(&new, &VAULT, false).is_err());
        assert!(prior::parse_current_layout(&new, &VAULT, true).is_err());
        assert!(
            prepare_declaration_data(&new, &VAULT).is_err(),
            "no redeclaration or supported4 opener is minted"
        );
    }
}

#[test]
fn descriptor_binds_vault_root_and_domain_separated_exact_original_three() {
    let old = original(None, 2);
    let new = prepare_declaration_data(&old, &VAULT).unwrap();
    let (_, expected) = original_data(&old, &VAULT).unwrap();
    let current_bytes = entries(&old).pop().unwrap().2;
    // Independent fixed canonical descriptor and complete hash preimage. Literal
    // length bytes pin u32 descriptor framing and u64 BIG-ENDIAN hash framing;
    // neither production descriptor encoding nor its digest helper builds this
    // expected value. A domain, framing or old-wire drift must fail this vector.
    let mut fixed =
        b"\0\0\0\x2acatcoms/private-chat-v2-current-layout/v1\0\0\x01\0\0\0\x20".to_vec();
    fixed.extend_from_slice(&[0x44; 32]);
    fixed.extend_from_slice(b"\0\0\0\x20");
    fixed.extend_from_slice(&[0x45; 32]);
    fixed.extend_from_slice(&[2, 1]);
    assert_eq!(fixed.len(), 122);
    assert_eq!(current_bytes, fixed);
    let mut preimage =
        b"catcoms/private-chat-v2-governance-current-descriptor/v1\0\0\0\0\0\0\0\0\x7a".to_vec();
    preimage.extend_from_slice(&fixed);
    assert_eq!(
        expected.current_descriptor,
        *blake3::hash(&preimage).as_bytes()
    );
    assert_ne!(expected.current_descriptor, outer::catalog_digest(&old));
    for changed in [
        GovernanceLayoutData {
            vault: [0; 32],
            ..expected
        },
        GovernanceLayoutData {
            root: [0; 32],
            ..expected
        },
        GovernanceLayoutData {
            current_descriptor: [0; 32],
            ..expected
        },
    ] {
        let mut values = entries(&new);
        values.last_mut().unwrap().2 = changed.encode_data().unwrap().to_vec();
        assert!(validate_declaration_pair_data(
            &old,
            &encoded(3, &outer::catalog_digest(&old), &values),
            &VAULT
        )
        .is_err());
    }
    let added = entries(&new).pop().unwrap().2;
    for end in 0..added.len() {
        assert!(GovernanceLayoutData::decode_data(&added[..end], expected).is_err());
    }
    let mut trailing = added;
    trailing.push(0);
    assert!(GovernanceLayoutData::decode_data(&trailing, expected).is_err());
    assert!(validate_declaration_pair_data(&old, &new, &[0; 32]).is_err());
}

#[test]
fn altered_prefix_unknown_or_duplicate_four_and_wrong_whole_parent_refuse() {
    let old = original(Some(2), 22);
    let new = prepare_declaration_data(&old, &VAULT).unwrap();
    for index in 0..3 {
        let mut values = entries(&new);
        *values[index].2.last_mut().unwrap() ^= 1;
        assert!(validate_declaration_pair_data(
            &old,
            &encoded(23, &outer::catalog_digest(&old), &values),
            &VAULT
        )
        .is_err());
    }
    for (id, version) in [(4, 2), (5, 1)] {
        let mut values = entries(&new);
        values.last_mut().unwrap().0 = id;
        values.last_mut().unwrap().1 = version;
        assert!(validate_declaration_pair_data(
            &old,
            &encoded(23, &outer::catalog_digest(&old), &values),
            &VAULT
        )
        .is_err());
    }
    let mut values = entries(&new);
    values.push(values.last().unwrap().clone());
    assert!(validate_declaration_pair_data(
        &old,
        &encoded(23, &outer::catalog_digest(&old), &values),
        &VAULT
    )
    .is_err());
    let values = entries(&new);
    for generation in [22, 24] {
        assert!(validate_declaration_pair_data(
            &old,
            &encoded(generation, &outer::catalog_digest(&old), &values),
            &VAULT
        )
        .is_err());
    }
    assert!(validate_declaration_pair_data(&old, &encoded(23, &[0; 32], &values), &VAULT).is_err());
}

#[test]
fn missing_original_three_legacy_unknown_overflow_and_caps_cannot_be_declarations() {
    assert!(prepare_declaration_data(&encode_empty(&VAULT).unwrap(), &VAULT).is_err());
    let old = original(None, 2);
    let mut values = entries(&old);
    values.pop();
    assert!(prepare_declaration_data(&encoded(2, &[0x4f; 32], &values), &VAULT).is_err());
    values.push((8, 1, vec![0]));
    assert!(prepare_declaration_data(&encoded(2, &[0x4f; 32], &values), &VAULT).is_err());
    assert!(prepare_declaration_data(&original(None, u64::MAX), &VAULT).is_err());
    let exact = outer::MAX_ENVELOPE_PLAIN - ENTRY_OVERHEAD - DESCRIPTOR_BYTES;
    // Arithmetic boundary only: no claim that such a synthetic length is a valid
    // existing catalog/source. The actual builder always validates its whole input.
    assert_eq!(candidate_len(exact).unwrap(), outer::MAX_ENVELOPE_PLAIN);
    assert!(candidate_len(exact + 1).is_err());
    assert!(candidate_len(usize::MAX).is_err());
    assert!(prepare_declaration_data(&vec![0; outer::MAX_ENVELOPE_PLAIN + 1], &VAULT).is_err());
}
