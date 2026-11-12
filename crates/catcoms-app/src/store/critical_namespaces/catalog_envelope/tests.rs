//! DATA-only syntax/policy separation. No namespace, mount, root or cleanup authority is minted.
use super::super::{encode_empty, pending_join_codec};
use super::*;
use catcoms_wire::Encoder;

const DOMAIN: [u8; 32] = [71; 32];
fn encoded(
    domain: &[u8],
    generation: u64,
    parent: &[u8],
    entries: &[(u32, u32, &[u8])],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(TAG_V2).unwrap();
    e.put_bytes(domain).unwrap();
    e.put_u64(generation);
    e.put_bytes(parent).unwrap();
    e.put_u32(entries.len() as u32);
    for (id, version, payload) in entries {
        e.put_u32(*id).put_u32(*version);
        e.put_bytes(payload).unwrap();
    }
    e.finish()
}

#[test]
fn exact_existing_legacy_empty_keeps_its_original_bytes_digest_and_negative_classification() {
    let bytes = encode_empty(&DOMAIN).unwrap();
    let parsed = parse(&bytes, &DOMAIN, 512).unwrap();
    assert!(parsed.format == Format::LegacyEmpty);
    assert_eq!(parsed.generation, 1);
    assert_eq!(parsed.parent, None);
    assert_eq!(parsed.count, 0);
    assert!(parsed.entry(0).is_err());
    assert!(parsed.entry(usize::MAX).is_err());
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/critical-catalog-plaintext/v1\0");
    hash.update(&bytes);
    assert_eq!(parsed.digest, *hash.finalize().as_bytes());
    let concrete = pending_join_codec::parse(&bytes, &DOMAIN).unwrap();
    assert_eq!(concrete.generation, 1);
    assert_eq!(concrete.count, 0);
    assert_eq!(concrete.digest, parsed.digest);
    assert!(parse(&bytes, &[72; 32], 512).is_err());
    let mut extra = bytes.to_vec();
    extra.push(0);
    assert!(parse(&extra, &DOMAIN, 512).is_err());
}

#[test]
fn unknown_and_archive_descriptors_parse_only_as_borrowed_data_and_join_policy_still_refuses() {
    let entries = [
        (1, 1, b"retained private source ciphertext".as_slice()),
        (2, 1, b"archive descriptor DATA".as_slice()),
    ];
    let bytes = encoded(&DOMAIN, 2, &[73; 32], &entries);
    let parsed = parse(&bytes, &DOMAIN, MAX_ENVELOPE_PLAIN).unwrap();
    assert!(parsed.format == Format::Namespaces);
    assert_eq!(parsed.count, 2);
    for (index, (id, version, payload)) in entries.iter().enumerate() {
        let entry = parsed.entry(index).unwrap();
        assert_eq!(entry.id, *id);
        assert_eq!(entry.version, *version);
        assert_eq!(entry.payload, *payload);
        assert!(entry.payload.as_ptr() >= bytes.as_ptr());
        assert!(entry.payload.as_ptr_range().end <= bytes.as_ptr_range().end);
        assert!(!format!("{entry:?}").contains("ciphertext"));
    }
    assert!(!format!("{parsed:?}").contains("private source"));
    assert!(
        pending_join_codec::parse(&bytes, &DOMAIN).is_err(),
        "shared syntax is NOT supported-handler admission"
    );
    for entries in [
        &[(2, 1, b"archive".as_slice())][..],
        &[(9, 99, b"future".as_slice())][..],
    ] {
        let bytes = encoded(&DOMAIN, 2, &[73; 32], entries);
        assert!(parse(&bytes, &DOMAIN, MAX_ENVELOPE_PLAIN).is_ok());
        assert!(pending_join_codec::parse(&bytes, &DOMAIN).is_err());
    }
}

#[test]
fn whole_plaintext_hash_changes_when_only_another_namespace_payload_changes() {
    let first = encoded(
        &DOMAIN,
        7,
        &[74; 32],
        &[(1, 1, b"same join payload"), (2, 1, b"source layout A")],
    );
    let second = encoded(
        &DOMAIN,
        7,
        &[74; 32],
        &[(1, 1, b"same join payload"), (2, 1, b"source layout B")],
    );
    let a = parse(&first, &DOMAIN, MAX_ENVELOPE_PLAIN).unwrap();
    let b = parse(&second, &DOMAIN, MAX_ENVELOPE_PLAIN).unwrap();
    assert_eq!(a.entry(0).unwrap().payload, b.entry(0).unwrap().payload);
    assert_eq!(a.parent, b.parent);
    assert_eq!(a.generation, b.generation);
    assert_ne!(a.digest, b.digest);
    assert_eq!(a.digest, catalog_digest(&first));
    assert_eq!(b.digest, catalog_digest(&second));
}

#[test]
fn strict_outer_limits_generation_ids_versions_lengths_and_eof_refuse_before_support() {
    let good = encoded(&DOMAIN, 2, &[75; 32], &[(2, 1, b"future payload")]);
    for end in 0..good.len() {
        assert!(parse(&good[..end], &DOMAIN, MAX_ENVELOPE_PLAIN).is_err());
    }
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(parse(&trailing, &DOMAIN, MAX_ENVELOPE_PLAIN).is_err());
    for generation in [0, 1] {
        assert!(parse(
            &encoded(&DOMAIN, generation, &[75; 32], &[(2, 1, b"x")]),
            &DOMAIN,
            MAX_ENVELOPE_PLAIN
        )
        .is_err());
    }
    for entries in [
        &[][..],
        &[(0, 1, b"".as_slice())][..],
        &[(1, 0, b"".as_slice())][..],
        &[(2, 1, b"".as_slice()), (2, 1, b"".as_slice())][..],
        &[(2, 1, b"".as_slice()), (1, 1, b"".as_slice())][..],
    ] {
        assert!(parse(
            &encoded(&DOMAIN, 2, &[75; 32], entries),
            &DOMAIN,
            MAX_ENVELOPE_PLAIN
        )
        .is_err());
    }
    assert!(parse(
        &encoded(&DOMAIN, 2, &[0; 31], &[(2, 1, b"x")]),
        &DOMAIN,
        MAX_ENVELOPE_PLAIN
    )
    .is_err());
    assert!(parse(
        &encoded(&[0; 31], 2, &[75; 32], &[(2, 1, b"x")]),
        &DOMAIN,
        MAX_ENVELOPE_PLAIN
    )
    .is_err());
    assert!(parse(&good, &DOMAIN, 0).is_err());
    assert!(parse(&good, &DOMAIN, good.len() - 1).is_err());
    assert!(parse(&good, &DOMAIN, MAX_ENVELOPE_PLAIN + 1).is_err());
    let mut oversized = vec![0; MAX_ENVELOPE_PLAIN + 1];
    oversized[..good.len()].copy_from_slice(&good);
    assert!(parse(&oversized, &DOMAIN, MAX_ENVELOPE_PLAIN).is_err());
    // Force an untrusted declared payload length beyond the actual complete candidate.
    let mut length = good.clone();
    let length_offset = good.len() - b"future payload".len() - 4;
    length[length_offset..length_offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(parse(&length, &DOMAIN, MAX_ENVELOPE_PLAIN).is_err());
    // A well-framed unknown OUTER format is not the known v2 envelope.
    let mut unknown = good.clone();
    let end = 4 + TAG_V2.len();
    unknown[end - 1] = b'3';
    assert!(parse(&unknown, &DOMAIN, MAX_ENVELOPE_PLAIN).is_err());
}

#[test]
fn fixed_entry_array_bounds_do_not_create_a_variable_namespace_container_or_empty_upgrade() {
    let entries: Vec<_> = (1..=MAX_ENTRIES)
        .map(|id| (id as u32, 1, b"".as_slice()))
        .collect();
    let bytes = encoded(&DOMAIN, u64::MAX, &[76; 32], &entries);
    let parsed = parse(&bytes, &DOMAIN, MAX_ENVELOPE_PLAIN).unwrap();
    assert_eq!(parsed.count, MAX_ENTRIES);
    for index in 0..MAX_ENTRIES {
        assert_eq!(parsed.entry(index).unwrap().id, index as u32 + 1);
    }
    assert!(parsed.entry(MAX_ENTRIES).is_err());
    let mut too_many = entries;
    too_many.push((MAX_ENTRIES as u32 + 1, 1, b"".as_slice()));
    assert!(parse(
        &encoded(&DOMAIN, 2, &[76; 32], &too_many),
        &DOMAIN,
        MAX_ENVELOPE_PLAIN
    )
    .is_err());
    // Legacy v1 can NEVER carry a namespace, even if every field has complete framing.
    let mut legacy = Encoder::new();
    legacy.put_str(TAG).unwrap();
    legacy.put_bytes(&DOMAIN).unwrap();
    legacy.put_u64(1).put_u32(1).put_u32(2).put_u32(1);
    legacy.put_bytes(b"archive").unwrap();
    assert!(parse(&legacy.finish(), &DOMAIN, MAX_ENVELOPE_PLAIN).is_err());
    assert_eq!(MAX_ENVELOPE_PLAIN, pending_join_codec::MAX_CATALOG_PLAIN);
    assert_eq!(
        OUTER_METADATA_BYTES,
        std::mem::size_of::<Envelope<'static>>()
    );
}
