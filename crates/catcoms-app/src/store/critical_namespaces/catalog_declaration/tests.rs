//! Outer-catalog DATA only: no namespace handler, mount or authority is exercised here.
use super::*;
use crate::store::critical_namespaces::encode_empty;

const DOMAIN: [u8; 32] = [0x61; 32];
type Entries = Vec<(u32, u32, Vec<u8>)>;

fn encoded(generation: u64, parent: &[u8; 32], entries: &Entries) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(outer::TAG_V2).unwrap();
    e.put_bytes(&DOMAIN).unwrap();
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
    let parsed = outer::parse(raw, &DOMAIN, outer::MAX_ENVELOPE_PLAIN).unwrap();
    (0..parsed.count)
        .map(|index| {
            let e = parsed.entry(index).unwrap();
            (e.id, e.version, e.payload.to_vec())
        })
        .collect()
}

fn base(ids: &[u32]) -> Entries {
    ids.iter()
        .map(|id| (*id, 1, vec![*id as u8; 3 + *id as usize]))
        .collect()
}

fn digest(raw: &[u8]) -> [u8; 32] {
    outer::catalog_digest(raw)
}

#[test]
fn insertion_above_every_id_is_byte_identical_to_the_older_append() {
    let original = encoded(7, &[0x62; 32], &base(&[1, 2, 3]));
    let candidate = insert_entry_data(&original, &DOMAIN, 5, 1, b"five").unwrap();
    let mut appended = base(&[1, 2, 3]);
    appended.push((5, 1, b"five".to_vec()));
    assert_eq!(
        candidate.as_slice(),
        encoded(8, &digest(&original), &appended).as_slice()
    );
}

#[test]
fn either_declaration_order_reaches_the_same_sorted_entries() {
    // 5 then 4.
    let start = encoded(3, &[0x63; 32], &base(&[2, 3]));
    let five = insert_entry_data(&start, &DOMAIN, 5, 1, b"member").unwrap();
    let both = insert_entry_data(&five, &DOMAIN, 4, 1, b"governance").unwrap();
    // 4 then 5.
    let four = insert_entry_data(&start, &DOMAIN, 4, 1, b"governance").unwrap();
    let both_other = insert_entry_data(&four, &DOMAIN, 5, 1, b"member").unwrap();
    let mut expected = base(&[2, 3]);
    expected.push((4, 1, b"governance".to_vec()));
    expected.push((5, 1, b"member".to_vec()));
    assert_eq!(entries(&both), expected);
    assert_eq!(entries(&both_other), expected);
    // Each step keeps its exact whole-plaintext parent and checked generation.
    assert_eq!(
        validate_insert_pair_data(&five, &both, &DOMAIN, 4, 1).unwrap(),
        b"governance"
    );
    let parsed = outer::parse(&both, &DOMAIN, outer::MAX_ENVELOPE_PLAIN).unwrap();
    assert_eq!(parsed.generation, 5);
    assert_eq!(parsed.parent, Some(digest(&five)));
}

#[test]
fn present_id_legacy_empty_full_table_and_zero_labels_refuse() {
    let original = encoded(4, &[0x64; 32], &base(&[2, 3, 5]));
    for version in [1, 2] {
        assert!(insert_entry_data(&original, &DOMAIN, 5, version, b"again").is_err());
    }
    assert!(insert_entry_data(&original, &DOMAIN, 0, 1, b"x").is_err());
    assert!(insert_entry_data(&original, &DOMAIN, 4, 0, b"x").is_err());
    let empty = encode_empty(&DOMAIN).unwrap();
    assert!(insert_entry_data(&empty, &DOMAIN, 4, 1, b"x").is_err());
    let ids: Vec<u32> = (1..=16).collect();
    let full = encoded(9, &[0x65; 32], &base(&ids));
    assert!(insert_entry_data(&full, &DOMAIN, 17, 1, b"x").is_err());
    let at_max = encoded(u64::MAX, &[0x66; 32], &base(&[2, 3]));
    assert!(insert_entry_data(&at_max, &DOMAIN, 4, 1, b"x").is_err());
    // A different domain is not this vault's catalog.
    assert!(insert_entry_data(&original, &[0x67; 32], 4, 1, b"x").is_err());
}

#[test]
fn dropped_changed_reordered_or_extra_entries_and_wrong_parent_refuse() {
    let original = encoded(4, &[0x68; 32], &base(&[1, 2, 3, 5]));
    let parent = digest(&original);
    let good = |set: &Entries| encoded(5, &parent, set);
    let mut expected = base(&[1, 2, 3]);
    expected.push((4, 1, b"four".to_vec()));
    expected.push(base(&[5]).remove(0));
    assert_eq!(
        validate_insert_pair_data(&original, &good(&expected), &DOMAIN, 4, 1).unwrap(),
        b"four"
    );

    // ID5 silently dropped while ID4 is added (count still +1 would need another entry).
    let mut dropped = expected.clone();
    dropped.remove(4);
    assert!(validate_insert_pair_data(&original, &good(&dropped), &DOMAIN, 4, 1).is_err());
    // ID5 payload changed.
    let mut changed = expected.clone();
    changed[4].2[0] ^= 1;
    assert!(validate_insert_pair_data(&original, &good(&changed), &DOMAIN, 4, 1).is_err());
    // ID5 version changed.
    let mut version = expected.clone();
    version[4].1 = 2;
    assert!(validate_insert_pair_data(&original, &good(&version), &DOMAIN, 4, 1).is_err());
    // ID5 replaced by an unknown ID6 at the same position.
    let mut replaced = expected.clone();
    replaced[4].0 = 6;
    assert!(validate_insert_pair_data(&original, &good(&replaced), &DOMAIN, 4, 1).is_err());
    // Wrong added version, or a different added ID than the handler's.
    assert!(validate_insert_pair_data(&original, &good(&expected), &DOMAIN, 4, 2).is_err());
    assert!(validate_insert_pair_data(&original, &good(&expected), &DOMAIN, 6, 1).is_err());
    // Two additions at once.
    let mut two = expected.clone();
    two.push((6, 1, b"six".to_vec()));
    assert!(validate_insert_pair_data(&original, &good(&two), &DOMAIN, 4, 1).is_err());
    // Unsorted and duplicate IDs never even parse.
    let mut unsorted = expected.clone();
    unsorted.swap(3, 4);
    assert!(validate_insert_pair_data(&original, &good(&unsorted), &DOMAIN, 4, 1).is_err());
    let mut duplicate = expected.clone();
    duplicate[3].0 = 5;
    assert!(validate_insert_pair_data(&original, &good(&duplicate), &DOMAIN, 4, 1).is_err());
    // Wrong parent and wrong generation.
    assert!(
        validate_insert_pair_data(&original, &encoded(5, &[0; 32], &expected), &DOMAIN, 4, 1)
            .is_err()
    );
    for generation in [4, 6] {
        assert!(validate_insert_pair_data(
            &original,
            &encoded(generation, &parent, &expected),
            &DOMAIN,
            4,
            1
        )
        .is_err());
    }
}
