//! Real original SDK/A/I fixture; all mutations are privately checked in Physical.
use super::*;

#[test]
fn genuine_i_rejects_authenticated_cross_plan_root_capture_quota_and_deletion_mutations() {
    let f = intent_fixture(0, 32571);
    let store = f.original.layout.checked_store().unwrap();
    f.intent.assert_original_i_semantic_rejections_for_test(
        store,
        &mut ChaCha20Rng::seed_from_u64(32572),
    );
}
