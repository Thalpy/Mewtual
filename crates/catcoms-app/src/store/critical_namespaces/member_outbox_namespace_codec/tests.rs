//! Namespace5 catalog DATA: declaration pairs in both orders with governance4, descriptor
//! binding, grammar selection and successor preservation. No mount, handler or authority.
use super::*;
use crate::store::critical_namespaces::{
    archive_bootstrap, catalog_envelope as outer, encode_empty, governance_namespace_codec,
};

const VAULT: [u8; 32] = [0x54; 32];
const ROOT: [u8; 32] = [0x55; 32];
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

/// One original Pending JOIN v1 row (DATA only), with chosen revision and scheduling.
fn join_v1(revision: u64, paused: bool) -> Vec<u8> {
    let mut immutable = Encoder::new();
    immutable.put_str(prior::IMMUTABLE_TAG).unwrap();
    immutable.put_bytes(&VAULT).unwrap();
    immutable.put_bytes(&[0x56; 32]).unwrap();
    immutable.put_u8(1).put_u8(1);
    for field in [
        &b"member-group"[..],
        &[0x57; 32],
        &[0x58; 32],
        &[0x59; 32],
        &[0x5a; 16],
        &[0x5b; 32],
        &[0x5c; 32],
        &[0x5d; 32],
    ] {
        immutable.put_bytes(field).unwrap();
    }
    immutable.put_bytes(&[0]).unwrap();
    let mut row = Encoder::new();
    row.put_bytes(&[0x56; 32]).unwrap();
    row.put_u64(revision).put_u8(u8::from(paused)).put_u8(1);
    row.put_bytes(&immutable.finish()).unwrap();
    row.put_bytes(&[0x5e; 40]).unwrap();
    let mut e = Encoder::new();
    e.put_str(prior::JOIN_TAG).unwrap();
    e.put_u32(1);
    e.put_bytes(&row.finish()).unwrap();
    e.finish()
}

fn archive_entry() -> (u32, u32, Vec<u8>) {
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
    )
}

fn current_entry() -> (u32, u32, Vec<u8>) {
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
    )
}

/// A settled Current3 catalog, optionally with a JOIN v1 entry.
fn current3(join: Option<Vec<u8>>, generation: u64) -> Vec<u8> {
    let mut values = Vec::new();
    if let Some(payload) = join {
        values.push((prior::JOIN_ID, prior::JOIN_VERSION, payload));
    }
    values.push(archive_entry());
    values.push(current_entry());
    encoded(generation, &[0x5f; 32], &values)
}

fn with_member(raw: &[u8]) -> prior::Parsed<'_> {
    prior::parse_configured_current_layout(raw, &VAULT, false, true).unwrap()
}

#[test]
fn declaration_inserts_only_five_and_only_the_configured_handler_reads_it() {
    for join in [None, Some(join_v1(3, true))] {
        let old = current3(join, 11);
        let new = prepare_declaration_data(&old, &VAULT).unwrap();
        let capability = validate_declaration_pair_data(&old, &new, &VAULT).unwrap();
        let next = outer::parse(&new, &VAULT, outer::MAX_ENVELOPE_PLAIN).unwrap();
        assert_eq!(next.generation, 12);
        assert_eq!(next.parent, Some(outer::catalog_digest(&old)));
        let old_entries = entries(&old);
        let new_entries = entries(&new);
        assert_eq!(&new_entries[..old_entries.len()], old_entries.as_slice());
        let added = new_entries.last().unwrap();
        assert_eq!((added.0, added.1), (5, 1));
        assert_eq!(added.2.len(), DESCRIPTOR_BYTES);

        // Every opener WITHOUT the concrete handler refuses namespace5 before cleanup.
        assert!(prior::parse(&new, &VAULT).is_err());
        assert!(prior::parse_bootstrap(&new, &VAULT).is_err());
        assert!(prior::parse_executing(&new, &VAULT).is_err());
        assert!(prior::parse_current_layout(&new, &VAULT, false).is_err());
        assert!(prior::parse_current_layout(&new, &VAULT, true).is_err());
        assert!(prior::parse_configured_current_layout(&new, &VAULT, true, false).is_err());
        // The configured handler reads exactly this capability and its exact payload.
        let parsed = with_member(&new);
        assert_eq!(parsed.member_outbox, Some(capability));
        assert_eq!(parsed.member_outbox_payload, Some(added.2.as_slice()));
        assert_eq!(
            capability.current_layout_data(),
            current::CurrentLayoutData {
                vault: VAULT,
                root: ROOT
            }
        );
        // No redeclaration, and ID5 never implies governance4.
        assert!(prepare_declaration_data(&new, &VAULT).is_err());
        assert!(parsed.governance.is_none());
    }
}

#[test]
fn either_order_with_governance_ends_at_the_same_two_exact_descriptors() {
    let old = current3(Some(join_v1(4, true)), 30);
    // 5 then 4.
    let five = prepare_declaration_data(&old, &VAULT).unwrap();
    let five_four = governance_namespace_codec::prepare_declaration_data(&five, &VAULT).unwrap();
    governance_namespace_codec::validate_declaration_pair_data(&five, &five_four, &VAULT).unwrap();
    // 4 then 5.
    let four = governance_namespace_codec::prepare_declaration_data(&old, &VAULT).unwrap();
    let four_five = prepare_declaration_data(&four, &VAULT).unwrap();
    validate_declaration_pair_data(&four, &four_five, &VAULT).unwrap();

    let ids = |raw: &[u8]| entries(raw).iter().map(|e| e.0).collect::<Vec<_>>();
    assert_eq!(ids(&five_four[..]), vec![1, 2, 3, 4, 5]);
    assert_eq!(entries(&five_four), entries(&four_five));
    // Each step preserved the earlier namespace byte for byte.
    assert_eq!(entries(&five).last(), entries(&five_four).last());
    assert_eq!(entries(&four).last(), Some(&entries(&four_five)[3]));

    // Grammar: governance4 + 5 needs BOTH concrete handlers; 5 alone needs only member.
    for raw in [&five_four[..], &four_five[..]] {
        assert!(prior::parse_configured_current_layout(raw, &VAULT, false, true).is_err());
        assert!(prior::parse_current_layout(raw, &VAULT, true).is_err());
    }
    // ID5 operation never creates or implies governance4.
    assert!(with_member(&five).governance.is_none());
}

#[test]
fn descriptor_binds_vault_root_and_exact_three_with_its_own_domain() {
    let old = current3(None, 2);
    let new = prepare_declaration_data(&old, &VAULT).unwrap();
    let expected = MemberOutboxCapabilityData::from_original_catalog_data(&old, &VAULT).unwrap();
    let three = current_entry().2;
    let mut preimage = b"catcoms/private-chat-v2-member-outbox-current-descriptor/v1\0".to_vec();
    preimage.extend_from_slice(&(three.len() as u64).to_be_bytes());
    preimage.extend_from_slice(&three);
    assert_eq!(
        expected.current_descriptor,
        *blake3::hash(&preimage).as_bytes()
    );
    for changed in [
        MemberOutboxCapabilityData {
            vault: [0; 32],
            ..expected
        },
        MemberOutboxCapabilityData {
            root: [0; 32],
            ..expected
        },
        MemberOutboxCapabilityData {
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
    // Governance's descriptor cannot occupy ID5, nor ID5's occupy ID4: distinct tags/domains.
    let governance = governance_namespace_codec::prepare_declaration_data(&old, &VAULT).unwrap();
    let mut swapped = entries(&new);
    swapped.last_mut().unwrap().2 = entries(&governance).pop().unwrap().2;
    assert!(validate_declaration_pair_data(
        &old,
        &encoded(3, &outer::catalog_digest(&old), &swapped),
        &VAULT
    )
    .is_err());
    let mut swapped = entries(&governance);
    swapped.last_mut().unwrap().2 = entries(&new).pop().unwrap().2;
    assert!(governance_namespace_codec::validate_declaration_pair_data(
        &old,
        &encoded(3, &outer::catalog_digest(&old), &swapped),
        &VAULT
    )
    .is_err());
    let added = entries(&new).pop().unwrap().2;
    for end in 0..added.len() {
        assert!(MemberOutboxCapabilityData::decode_exact(&added[..end], expected).is_err());
    }
    let mut trailing = added;
    trailing.push(0);
    assert!(MemberOutboxCapabilityData::decode_exact(&trailing, expected).is_err());
    assert!(validate_declaration_pair_data(&old, &new, &[0; 32]).is_err());
}

#[test]
fn unsupported_version_duplicate_unsorted_and_missing_three_refuse() {
    let old = current3(None, 5);
    let new = prepare_declaration_data(&old, &VAULT).unwrap();
    // Unsupported ID5 version, through the configured handler and the pair validator.
    let mut version = entries(&new);
    version.last_mut().unwrap().1 = 2;
    let versioned = encoded(6, &outer::catalog_digest(&old), &version);
    assert!(prior::parse_configured_current_layout(&versioned, &VAULT, false, true).is_err());
    assert!(validate_declaration_pair_data(&old, &versioned, &VAULT).is_err());
    // Duplicate and unsorted namespace IDs never parse.
    let mut duplicate = entries(&new);
    duplicate.push(duplicate.last().unwrap().clone());
    assert!(prior::parse_configured_current_layout(
        &encoded(6, &outer::catalog_digest(&old), &duplicate),
        &VAULT,
        false,
        true
    )
    .is_err());
    let mut unsorted = entries(&new);
    let five = unsorted.pop().unwrap();
    unsorted.insert(1, five);
    assert!(prior::parse_configured_current_layout(
        &encoded(6, &outer::catalog_digest(&old), &unsorted),
        &VAULT,
        false,
        true
    )
    .is_err());
    // No ID3 (or a legacy empty catalog) cannot carry or declare namespace5.
    let mut no_three = entries(&old);
    no_three.pop();
    assert!(prepare_declaration_data(&encoded(5, &[0x5f; 32], &no_three), &VAULT).is_err());
    let mut orphan = no_three.clone();
    orphan.push(entries(&new).pop().unwrap());
    assert!(prior::parse_configured_current_layout(
        &encoded(6, &[0x5f; 32], &orphan),
        &VAULT,
        false,
        true
    )
    .is_err());
    assert!(prepare_declaration_data(&encode_empty(&VAULT).unwrap(), &VAULT).is_err());
    assert!(prepare_declaration_data(&current3(None, u64::MAX), &VAULT).is_err());
}

#[test]
fn ordinary_join_successors_must_keep_five_and_its_exact_payload() {
    // Current: paused row revision 3 with ID5. A legal toggle keeps everything else.
    let old = current3(Some(join_v1(3, true)), 40);
    let declared = prepare_declaration_data(&old, &VAULT).unwrap();
    let current = with_member(&declared).summary([1; 32]);
    let parent = outer::catalog_digest(&declared);
    let mut toggled = entries(&declared);
    toggled[0].2 = join_v1(4, false);
    let next_raw = encoded(42, &parent, &toggled);
    let next = with_member(&next_raw).summary([2; 32]);
    prior::successor(&current, &next).unwrap();

    // The same toggle that silently drops ID5 refuses.
    let mut dropped = toggled.clone();
    dropped.pop();
    let dropped_raw = encoded(42, &parent, &dropped);
    let dropped = with_member(&dropped_raw).summary([3; 32]);
    assert!(prior::successor(&current, &dropped).is_err());
    // A toggle that also "re-adds" ID5 to a catalog that lacked it refuses too.
    let plain = with_member(&old).summary([4; 32]);
    let mut readded = toggled.clone();
    readded[0].2 = join_v1(4, false);
    let readded_raw = encoded(41, &outer::catalog_digest(&old), &readded);
    let readded = with_member(&readded_raw).summary([5; 32]);
    assert!(prior::successor(&plain, &readded).is_err());
}
