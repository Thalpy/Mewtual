//! All original SDK/I/raw bytes come from the real selected-I signing guard.
use super::*;

#[test]
fn genuine_s_rejects_authenticated_cross_plan_capture_carry_counter_and_all_signature_field_mutations(
) {
    let f = raw_fixture(0, 0x7fe80);
    let store = f.original.layout.checked_store().unwrap();
    f.signature.assert_original_s_semantic_rejections_for_test(
        store,
        &mut ChaCha20Rng::seed_from_u64(0x7fe81),
    );
}
