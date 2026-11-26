use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::fs;

#[test]
fn governed_add_fence_same_handle_refuses_in_place_edits_and_equal_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(STAGE);
    let parked = directory.path().join("original.held");
    let raw = GovernedAddFenceData {
        context: TextContextIdData::from_bytes([1; 32]),
        parent: TextHeadIdData::from_bytes([2; 32]),
        invite: [3; 32],
        request: [4; 32],
    }
    .seal_data(&[5; 32], &mut ChaCha20Rng::seed_from_u64(6))
    .unwrap();
    fs::write(&path, raw.as_slice()).unwrap();
    let mut original = read_control(&path, FRAME_BYTES).unwrap();
    require_exact_held(&path, &mut original).unwrap();

    let mut changed = raw.to_vec();
    changed[FRAME_BYTES - 1] ^= 1;
    fs::write(&path, changed).unwrap();
    assert!(require_exact_held(&path, &mut original).is_err());
    fs::write(&path, raw.as_slice()).unwrap();
    require_exact_held(&path, &mut original).unwrap();

    fs::rename(&path, &parked).unwrap();
    fs::write(&path, raw.as_slice()).unwrap();
    assert!(require_exact_held(&path, &mut original).is_err());
    fs::remove_file(&path).unwrap();
    fs::rename(&parked, &path).unwrap();
    require_exact_held(&path, &mut original).unwrap();
}
