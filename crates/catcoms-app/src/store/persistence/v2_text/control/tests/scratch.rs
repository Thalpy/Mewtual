//! MAC classification is DATA only. Actual reserved-purpose/parent/NoDeps/Current/lease
//! admission is pinned separately by the genuine promotion recovery tests.
use super::super::*;
use catcoms_crypto::seal;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn authenticated(kind: TextControlKindData, version: u16) -> Vec<u8> {
    let mut e = catcoms_wire::Encoder::new();
    e.put_bytes(kind.domains().0).unwrap();
    e.put_u16(version);
    let key = blake3::derive_key(kind.domains().1, &[71; 32]);
    crate::store::frame(&seal(&key, &e.finish(), &mut ChaCha20Rng::seed_from_u64(72)).unwrap())
}

#[test]
fn authenticated_foreign_head_archive_and_unknown_seed_versions_are_preserved() {
    for (kind, version) in [
        (TextControlKindData::Head, 1),
        (TextControlKindData::Archive, 1),
        (TextControlKindData::Seed, 2),
        (TextControlKindData::Wal, 77),
    ] {
        let raw = authenticated(kind, version);
        assert!(matches!(
            inspect_seed_stage_mac_data(&[71; 32], &raw).unwrap(),
            TextControlMacInspectionData::Authenticated
        ));
    }
}

#[test]
fn authenticated_empty_plaintext_is_preserved_even_when_the_normal_parser_refuses_it() {
    let key = blake3::derive_key(TextControlKindData::Seed.domains().1, &[71; 32]);
    let raw = crate::store::frame(&seal(&key, b"", &mut ChaCha20Rng::seed_from_u64(73)).unwrap());
    assert_eq!(raw.len(), 40);
    assert!(matches!(
        inspect_seed_stage_mac_data(&[71; 32], &raw).unwrap(),
        TextControlMacInspectionData::Authenticated
    ));
}

#[test]
fn bounded_torn_mac_failures_are_not_absence_and_oversize_unknown_envelopes_refuse() {
    let raw = authenticated(TextControlKindData::Seed, 1);
    for end in [0, 1, 23, 24, 39, raw.len() - 1] {
        assert!(matches!(
            inspect_seed_stage_mac_data(&[71; 32], &raw[..end]).unwrap(),
            TextControlMacInspectionData::Unauthenticated
        ));
    }
    assert!(inspect_seed_stage_mac_data(&[71; 32], &vec![0; 32768 + 41]).is_err());
    let domain = b"catcoms/private-chat-sealed-page/v1\0";
    let mut private_page = (domain.len() as u32).to_be_bytes().to_vec();
    private_page.extend_from_slice(domain);
    assert!(inspect_seed_stage_mac_data(&[71; 32], &private_page).is_err());
}
