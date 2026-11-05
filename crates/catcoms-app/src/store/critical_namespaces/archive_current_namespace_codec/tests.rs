//! Real AEAD/strict DATA framing only. No paid/Saved/Current fixture or supported ID3 mount.
//! Authored UNRUN while compiler, test and build execution is paused.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, Error, RngCore, SeedableRng};
use std::{cell::Cell, ops::Range};

const KEY: [u8; 32] = [63; 32];
fn layout() -> CurrentLayoutData {
    CurrentLayoutData {
        vault: [61; 32],
        root: [62; 32],
    }
}
struct CountingRng {
    inner: ChaCha20Rng,
    bytes: usize,
}
impl CountingRng {
    fn new() -> Self {
        Self {
            inner: ChaCha20Rng::seed_from_u64(60),
            bytes: 0,
        }
    }
}
impl RngCore for CountingRng {
    fn next_u32(&mut self) -> u32 {
        self.bytes += 4;
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.bytes += 8;
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        self.bytes += out.len();
        self.inner.fill_bytes(out);
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), Error> {
        self.bytes += out.len();
        self.inner.try_fill_bytes(out)
    }
}
impl CryptoRng for CountingRng {}
fn fixture() -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
    let marker = seal_marker_data(&KEY, layout(), &mut CountingRng::new()).unwrap();
    // Opaque catalog bytes intentionally do NOT constitute a catalog/parent/SourceSaved proof.
    let candidate = vec![0xa5; 64];
    let seed = encode_declaration_seed_data(&KEY, layout(), [64; 32], &marker, &candidate).unwrap();
    (marker, seed)
}
fn ranges(raw: &[u8]) -> Vec<Range<usize>> {
    let mut d = Decoder::new(raw);
    d.get_bytes().unwrap();
    d.get_u16().unwrap();
    let mut ranges = Vec::new();
    for _ in 0..5 {
        let start = raw.len() - d.remaining() + 4;
        let bytes = d.get_bytes().unwrap();
        ranges.push(start..start + bytes.len());
    }
    d.finish().unwrap();
    ranges
}

#[test]
fn literal_layout_geometry_role_version_complete_consumption_and_expected_context() {
    let raw = layout().encode_data().unwrap();
    assert_eq!(raw.len(), 122);
    assert_eq!(
        CurrentLayoutData::decode_data(&raw, layout()).unwrap(),
        layout()
    );
    assert_eq!(NAMESPACE_ID, 3);
    assert_eq!(NAMESPACE_VERSION, 1);
    assert_eq!(MAX_LAYOUT_PLAIN, 512);
    assert_eq!(MAX_MARKER_FRAME, 1064);
    assert_eq!(MAX_CATALOG_FRAME, 16 * 1024 * 1024 + 4096 + 512 + 40);
    assert_eq!(MAX_DECLARATION_FRAME, MAX_CATALOG_FRAME + 4136);
    assert_eq!(DECLARATION_FEE, 12 * MAX_DECLARATION_FRAME + 256 * 1024);
    assert_eq!(NAMESPACE_METADATA_FEE, 64 * 1024);
    let mut d = Decoder::new(&raw);
    assert_eq!(d.get_bytes().unwrap(), LAYOUT_TAG);
    assert_eq!(d.get_u16().unwrap(), 1);
    assert_eq!(d.get_bytes().unwrap(), layout().vault.as_slice());
    assert_eq!(d.get_bytes().unwrap(), layout().root.as_slice());
    assert_eq!(d.get_u8().unwrap(), 2);
    assert_eq!(d.get_u8().unwrap(), 1);
    d.finish().unwrap();
    for len in 0..raw.len() {
        assert!(CurrentLayoutData::decode_data(&raw[..len], layout()).is_err());
    }
    for at in [0, 4, 4 + LAYOUT_TAG.len(), raw.len() - 2, raw.len() - 1] {
        let mut changed = raw.to_vec();
        changed[at] ^= 1;
        assert!(CurrentLayoutData::decode_data(&changed, layout()).is_err());
    }
    assert!(CurrentLayoutData::decode_data(&[raw.as_slice(), &[0]].concat(), layout()).is_err());
    assert!(CurrentLayoutData::decode_data(&vec![0; MAX_LAYOUT_PLAIN + 1], layout()).is_err());
    for expected in [
        CurrentLayoutData {
            vault: [1; 32],
            ..layout()
        },
        CurrentLayoutData {
            root: [1; 32],
            ..layout()
        },
    ] {
        assert!(CurrentLayoutData::decode_data(&raw, expected).is_err());
    }
}

#[test]
fn group_path_is_lower_hex_group_wide_and_bound_to_full_vault_root_and_group() {
    let group = [31; 256];
    let name = group_name_data(layout(), &group).unwrap();
    assert_eq!(name.len(), 64);
    assert!(name
        .iter()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)));
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/private-chat-v2-group-root/v1\0");
    h.update(&layout().vault);
    h.update(&layout().root);
    h.update(&256u32.to_be_bytes());
    h.update(&group);
    assert_eq!(
        name.as_slice(),
        hex::encode(h.finalize().as_bytes()).as_bytes()
    );
    assert_eq!(
        name,
        group_name_data(layout(), &group).unwrap(),
        "no local signer parameter can split the group fence"
    );
    assert_ne!(
        name,
        group_name_data(
            CurrentLayoutData {
                vault: [1; 32],
                ..layout()
            },
            &group
        )
        .unwrap()
    );
    assert_ne!(
        name,
        group_name_data(
            CurrentLayoutData {
                root: [1; 32],
                ..layout()
            },
            &group
        )
        .unwrap()
    );
    assert_ne!(name, group_name_data(layout(), &group[..255]).unwrap());
    assert!(group_name_data(layout(), &[]).is_err());
    assert!(group_name_data(layout(), &[0; 257]).is_err());
}

#[test]
fn marker_is_actual_tagged_aead_and_key_root_domain_and_frame_changes_refuse() {
    let mut rng = CountingRng::new();
    let marker = seal_marker_data(&KEY, layout(), &mut rng).unwrap();
    assert_eq!(marker.len(), 174 + 40);
    assert_eq!(rng.bytes, 24);
    open_marker_data(&KEY, layout(), &marker).unwrap();
    assert!(open_marker_data(&[0; 32], layout(), &marker).is_err());
    assert!(open_marker_data(
        &KEY,
        CurrentLayoutData {
            root: [1; 32],
            ..layout()
        },
        &marker
    )
    .is_err());
    assert!(
        open_declaration_seed_data(&KEY, &marker).is_err(),
        "separate KDF domains"
    );
    for at in [0, 24, marker.len() - 1] {
        let mut changed = marker.to_vec();
        changed[at] ^= 1;
        assert!(open_marker_data(&KEY, layout(), &changed).is_err());
    }
    for len in 0..marker.len() {
        assert!(open_marker_data(&KEY, layout(), &marker[..len]).is_err());
    }
    assert!(open_marker_data(&KEY, layout(), &vec![0; MAX_MARKER_FRAME + 1]).is_err());
    let again = seal_marker_data(&KEY, layout(), &mut rng).unwrap();
    assert_eq!(rng.bytes, 48);
    assert_ne!(marker.as_slice(), again.as_slice());
    // The DATA codec itself does not promise retry caching; the future physical owner must
    // retain the first exact candidate, rather than calling this encoder a second time.
}

#[test]
fn declaration_preserves_complete_opaque_candidates_and_exact_parent_label_as_data_only() {
    let (marker, plain) = fixture();
    let parsed = parse_declaration_seed_data(&KEY, layout(), &plain).unwrap();
    assert_eq!(parsed.expected_catalog_id, [64; 32]);
    assert_eq!(parsed.marker_frame, marker.as_slice());
    assert_eq!(parsed.catalog_frame, &[0xa5; 64]);
    for bytes in [parsed.marker_frame, parsed.catalog_frame] {
        assert!(bytes.as_ptr() as usize >= plain.as_ptr() as usize);
        assert!(bytes.as_ptr() as usize + bytes.len() <= plain.as_ptr() as usize + plain.len());
    }
    let mut rng = CountingRng::new();
    let sealed = seal_declaration_seed_data(&KEY, &plain, &mut rng).unwrap();
    assert_eq!(rng.bytes, 24);
    assert_eq!(
        open_declaration_seed_data(&KEY, &sealed)
            .unwrap()
            .as_slice(),
        plain.as_slice()
    );
    assert!(open_marker_data(&KEY, layout(), &sealed).is_err());
    let other_parent =
        encode_declaration_seed_data(&KEY, layout(), [65; 32], &marker, &[0xa5; 64]).unwrap();
    assert_eq!(
        parse_declaration_seed_data(&KEY, layout(), &other_parent)
            .unwrap()
            .expected_catalog_id,
        [65; 32],
        "parent equality/file barriers belong to the actual writer; arbitrary labels stay DATA"
    );
    assert!(parse_declaration_seed_data(
        &KEY,
        CurrentLayoutData {
            vault: [1; 32],
            ..layout()
        },
        &plain
    )
    .is_err());
    assert!(parse_declaration_seed_data(
        &KEY,
        CurrentLayoutData {
            root: [1; 32],
            ..layout()
        },
        &plain
    )
    .is_err());
}

#[test]
fn every_malformed_late_declaration_frame_refuses_before_nested_marker_open() {
    let (_marker, plain) = fixture();
    let spans = ranges(&plain);
    let mut cases = Vec::new();
    for span in &spans {
        let mut changed = plain.to_vec();
        changed[span.start - 4..span.start].copy_from_slice(&u32::MAX.to_be_bytes());
        cases.push(changed);
    }
    cases.push([plain.as_slice(), &[0]].concat());
    for len in 0..plain.len() {
        MARKER_OPEN_ENTRIES.with(|n| n.set(0));
        assert!(parse_declaration_seed_data(&KEY, layout(), &plain[..len]).is_err());
        assert_eq!(MARKER_OPEN_ENTRIES.with(Cell::get), 0);
    }
    for bad in cases {
        MARKER_OPEN_ENTRIES.with(|n| n.set(0));
        assert!(parse_declaration_seed_data(&KEY, layout(), &bad).is_err());
        assert_eq!(MARKER_OPEN_ENTRIES.with(Cell::get), 0);
    }
    MARKER_OPEN_ENTRIES.with(|n| n.set(0));
    parse_declaration_seed_data(&KEY, layout(), &plain).unwrap();
    assert_eq!(
        MARKER_OPEN_ENTRIES.with(Cell::get),
        1,
        "positive control reaches actual nested opener"
    );
}

#[test]
fn maximum_catalog_candidate_roundtrips_and_oversize_is_rejected_before_randomness() {
    let marker = seal_marker_data(&KEY, layout(), &mut CountingRng::new()).unwrap();
    let candidate = vec![0xa5; MAX_CATALOG_FRAME];
    let plain =
        encode_declaration_seed_data(&KEY, layout(), [64; 32], &marker, &candidate).unwrap();
    assert_eq!(plain.len(), MAX_CATALOG_FRAME + 383);
    let parsed = parse_declaration_seed_data(&KEY, layout(), &plain).unwrap();
    assert_eq!(parsed.catalog_frame, candidate.as_slice());
    let mut rng = CountingRng::new();
    let sealed = seal_declaration_seed_data(&KEY, &plain, &mut rng).unwrap();
    assert_eq!(sealed.len(), plain.len() + 40);
    assert!(sealed.len() <= MAX_DECLARATION_FRAME);
    assert_eq!(
        open_declaration_seed_data(&KEY, &sealed)
            .unwrap()
            .as_slice(),
        plain.as_slice()
    );
    assert_eq!(rng.bytes, 24);
    assert!(encode_declaration_seed_data(
        &KEY,
        layout(),
        [64; 32],
        &marker,
        &vec![0; MAX_CATALOG_FRAME + 1]
    )
    .is_err());
    assert!(encode_declaration_seed_data(&KEY, layout(), [64; 32], &marker, &[0; 40]).is_err());
    assert!(
        seal_declaration_seed_data(&KEY, &vec![0; MAX_DECLARATION_PLAIN + 1], &mut rng).is_err()
    );
    assert_eq!(rng.bytes, 24, "oversize cannot consume another nonce");
    assert!(open_declaration_seed_data(&KEY, &vec![0; MAX_DECLARATION_FRAME + 1]).is_err());
}

#[test]
fn authenticated_malformed_marker_and_seed_remain_refused_and_debug_omits_identifiers() {
    let (marker, plain) = fixture();
    let derived = Zeroizing::new(blake3::derive_key(
        "catcoms/private-chat-v2-current-marker-key/v1",
        &KEY,
    ));
    let malformed = Zeroizing::new(frame(
        &seal(&derived, b"not the marker grammar", &mut CountingRng::new()).unwrap(),
    ));
    assert!(open_marker_data(&KEY, layout(), &malformed).is_err());
    assert!(
        encode_declaration_seed_data(&KEY, layout(), [64; 32], &malformed, &[0xa5; 64]).is_err()
    );
    let spans = ranges(&plain);
    let mut changed = plain.to_vec();
    changed[spans[3].start] ^= 1;
    let sealed = seal_declaration_seed_data(&KEY, &changed, &mut CountingRng::new()).unwrap();
    let opened = open_declaration_seed_data(&KEY, &sealed).unwrap();
    assert!(parse_declaration_seed_data(&KEY, layout(), &opened).is_err());
    let parsed = parse_declaration_seed_data(&KEY, layout(), &plain).unwrap();
    for debug in [format!("{:?}", layout()), format!("{parsed:?}")] {
        assert!(!debug.contains(&hex::encode(layout().vault)));
        assert!(!debug.contains(&hex::encode(layout().root)));
        assert!(!debug.contains(&hex::encode(&marker[..24])));
    }
}
