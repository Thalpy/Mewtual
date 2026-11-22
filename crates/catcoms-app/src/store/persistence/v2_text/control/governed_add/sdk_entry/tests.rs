use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_sdk_entry_binds_the_original_fence_ciphertext() {
    let key = [5; 32];
    let data = GovernedAddFenceData {
        context: TextContextIdData::from_bytes([1; 32]),
        parent: TextHeadIdData::from_bytes([2; 32]),
        invite: [3; 32],
        request: [4; 32],
    };
    let mut rng = ChaCha20Rng::seed_from_u64(63);
    let fence = data.seal_data(&key, &mut rng).unwrap();
    let new_fence = data.seal_data(&key, &mut rng).unwrap();
    assert_ne!(fence.as_slice(), new_fence.as_slice());
    assert!(GovernedAddFenceData::open_data(&key, &new_fence).unwrap() == data);
    let expected = GovernedAddSdkEntryData::from_fence_data(&fence).unwrap();
    let raw = expected.seal_data(&key, &mut rng).unwrap();
    expected.require_fence_data(&key, &raw).unwrap();
    assert!(GovernedAddSdkEntryData::from_fence_data(&new_fence)
        .unwrap()
        .require_fence_data(&key, &raw)
        .is_err());
    assert!(expected.require_fence_data(&[6; 32], &raw).is_err());
    assert!(expected.require_fence_data(&key, &fence).is_err());
    for length in 0..raw.len() {
        assert!(expected.require_fence_data(&key, &raw[..length]).is_err());
    }
    let mut trailer = raw.to_vec();
    trailer.push(0);
    assert!(expected.require_fence_data(&key, &trailer).is_err());
}
