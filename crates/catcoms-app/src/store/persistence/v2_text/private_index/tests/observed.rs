//! Exact actual cipher lengths are not a maximum or a caller's suggested size.
use super::*;
#[test]
fn authenticated_private_path_records_actual_cipher_bytes_only_after_all_checks() {
    let mut rng = ChaCha20Rng::seed_from_u64(0x9381);
    let c = context(PrivateIndexKindData::Token);
    let row = token(31, RequestPhaseData::Retained);
    let (page, reference) = sealed(c, &[row], NodePlacementData::Root, &mut rng);
    let mut path = PrivateIndexPathData::new_data(c, reference.node, row.key).unwrap();
    assert_eq!(path.original_frame_sizes_data().len(), 0);
    let mut wrong = page.as_bytes().to_vec();
    let at = wrong.len() / 2;
    wrong[at] ^= 1;
    assert!(path.accept_frame_data(&KEY, &wrong).is_err());
    assert_eq!(path.original_frame_sizes_data().len(), 0);
    path.accept_frame_data(&KEY, page.as_bytes()).unwrap();
    let actual: Vec<_> = path.original_frame_sizes_data().collect();
    assert_eq!(actual, vec![(reference.node, page.as_bytes().len())]);
    assert!(page.as_bytes().len() < MAX_INDEX_FRAME_BYTES);
    let found = path.row_data().unwrap().unwrap();
    assert_eq!(found.key, row.key);
    assert_eq!(found.value, row.value);
}
