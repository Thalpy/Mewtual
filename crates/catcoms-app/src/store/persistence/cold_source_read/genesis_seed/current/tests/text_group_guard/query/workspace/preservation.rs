//! Real first-predecessor storage refusals. Authenticated tampered controls model an
//! inconsistent/private-prototype filesystem; no Saved, semantic append or Current owner
//! is fabricated. The actual query/Current/lane/read paths must still refuse new drafts.
use super::*;
use catcoms_wire::Decoder;
use rand_core::SeedableRng;

#[test]
fn byte_different_initial_selector_cannot_replace_the_seed_bound_clear_predecessor() {
    let original = super::super::super::promotion::promoted_text_fixture(0, 0x7f930);
    let store = original.layout.checked_store().unwrap();
    let selected = selected_capture(&original);
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().original_target_data().0.to_owned();
    let head_path = target.join("head.bin");
    let old = std::fs::read(&head_path).unwrap();
    let key = store.keys.db_key().unwrap();
    let head_key = blake3::derive_key("catcoms/private-chat-text-head-key/v1", &key);
    let plain = catcoms_crypto::unseal(&head_key, &crate::store::unframe(&old).unwrap()).unwrap();
    // No actual later-B writer exists in this cohort. Instead simulate an inconsistent
    // authenticated replacement of O_G using a different private envelope, with exactly
    // the SAME public/source/semantic fields. This is not a Saved or selected-B receipt.
    // The real later-B + missing-WAL regression remains required with that real producer.
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0x7f931);
    let changed = crate::store::frame(&catcoms_crypto::seal(&head_key, &plain, &mut rng).unwrap());
    assert_ne!(changed, old);
    std::fs::write(&head_path, &changed).unwrap();
    assert!(!target.join("text.wal").exists());
    drop(owner);
    drop(selected);

    let selected = selected_capture(&original);
    let query = completed_query(&original, &selected, [31; 32]);
    let (_producer, mut prepared) = workspace(&original, &selected, &query, "must not draft");
    let lane = create_lane(&original, prepared.resources());
    let admitted = read_workspace(&original, &lane, &mut prepared, &query);
    assert!(prepared.run_read(admitted).is_err());
    assert!(!prepared.has_ready_data());
    assert!(prepared.ready_clear().is_err());
    assert_eq!(std::fs::read(head_path).unwrap(), changed);
    assert!(!target.join("text.wal").exists());
}

/// Sign/export/Clock are not involved in any storage refusal below. This RNG is used only
/// by the test's explicitly hostile private-control encoder, never by a production draft.
fn reseal_seed(raw: &[u8], key: &[u8; 32], mutate: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let derived = blake3::derive_key("catcoms/private-chat-text-seed-key/v1", key);
    let mut plain = catcoms_crypto::unseal(&derived, &crate::store::unframe(raw).unwrap()).unwrap();
    mutate(&mut plain);
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0x7f941);
    crate::store::frame(&catcoms_crypto::seal(&derived, &plain, &mut rng).unwrap())
}

#[test]
fn corrupt_wrong_context_and_future_seed_controls_are_preserved_without_ready() {
    for case in 0..5 {
        let original = super::super::super::promotion::promoted_text_fixture(0, 0x7f940 + case);
        let selected = selected_capture(&original);
        let query = completed_query(&original, &selected, [31; 32]);
        let (_producer, mut prepared) = workspace(&original, &selected, &query, "no draft");
        let lane = create_lane(&original, prepared.resources());
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().original_target_data().0.to_owned();
        let path = target.join("text.seed");
        let original_seed = std::fs::read(&path).unwrap();
        let key = store.keys.db_key().unwrap();
        let hostile = match case {
            0 => {
                let mut bytes = original_seed.clone();
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                bytes
            }
            1 => reseal_seed(&original_seed, &key, |plain| {
                // Context C is the first bytes field after tag/version. Its nonce is
                // the tenth b32, so change a real C field under an otherwise valid MAC.
                let mut d = Decoder::new(plain);
                d.get_bytes().unwrap();
                d.get_u16().unwrap();
                let c = d.get_bytes().unwrap();
                let c_at = plain.len() - d.remaining() - c.len();
                let mut cd = Decoder::new(c);
                cd.get_bytes().unwrap();
                cd.get_u16().unwrap();
                for _ in 0..9 {
                    cd.get_bytes().unwrap();
                }
                let nonce_at = c.len() - cd.remaining() + 4;
                plain[c_at + nonce_at] ^= 1;
            }),
            2 => reseal_seed(&original_seed, &key, |plain| {
                let mut d = Decoder::new(plain);
                d.get_bytes().unwrap();
                let version_at = plain.len() - d.remaining();
                plain[version_at..version_at + 2].copy_from_slice(&2u16.to_be_bytes());
            }),
            3 => std::fs::read(target.join("head.bin")).unwrap(),
            4 => Vec::new(),
            _ => unreachable!(),
        };
        std::fs::write(&path, &hostile).unwrap();
        drop(owner);
        let admitted = read_workspace(&original, &lane, &mut prepared, &query);
        assert!(prepared.run_read(admitted).is_err());
        assert!(!prepared.has_ready_data());
        assert_eq!(std::fs::read(path).unwrap(), hostile);
        assert!(!target.join("text.wal").exists());
    }
}
