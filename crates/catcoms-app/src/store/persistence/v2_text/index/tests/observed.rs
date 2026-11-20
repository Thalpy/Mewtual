//! DATA-only observation checks with actual ciphertexts. No synthetic page is a
//! selected root, accepted miss, Current role or retirement authority in these tests.
use super::*;

#[test]
fn observed_frame_lengths_appear_only_after_full_authentication_and_are_exact_cipher_lengths() {
    let mut rng = ChaCha20Rng::seed_from_u64(0x7fe00);
    let raw = leaf(10, 1, NodePlacementData::Root);
    let (page, root) = seal(&raw, NodePlacementData::Root, &mut rng);
    let mut path = AcceptedIndexPathData::new_data(context(), root, key(11), None).unwrap();
    assert_eq!(path.original_frame_sizes_data().len(), 0);
    assert!(path.accept_frame_data(&[0; 32], page.as_bytes()).is_err());
    assert_eq!(path.original_frame_sizes_data().len(), 0);
    let mut changed = page.as_bytes().to_vec();
    let at = changed.len() / 2;
    changed[at] ^= 0x20;
    assert!(path.accept_frame_data(&KEY, &changed).is_err());
    assert_eq!(path.original_frame_sizes_data().len(), 0);
    path.accept_frame_data(&KEY, page.as_bytes()).unwrap();
    let expected = vec![(root, page.as_bytes().len())];
    assert_eq!(
        path.original_frame_sizes_data().collect::<Vec<_>>(),
        expected
    );
    assert!(path.accept_frame_data(&KEY, page.as_bytes()).is_err());
    assert_eq!(
        path.original_frame_sizes_data().collect::<Vec<_>>(),
        expected
    );

    let mut foreign_context = context();
    foreign_context.vault = [99; 32];
    let foreign_raw =
        encode_leaf_data(foreign_context, &[row(10)], NodePlacementData::Root).unwrap();
    let (foreign, foreign_root) = seal(&foreign_raw, NodePlacementData::Root, &mut rng);
    let mut wrong_context =
        AcceptedIndexPathData::new_data(context(), foreign_root, key(11), None).unwrap();
    // ID and AEAD both match this genuine encrypted frame, but its authenticated
    // context differs. No observation is cached before that final codec boundary.
    assert!(wrong_context
        .accept_frame_data(&KEY, foreign.as_bytes())
        .is_err());
    assert_eq!(wrong_context.original_frame_sizes_data().len(), 0);
}
