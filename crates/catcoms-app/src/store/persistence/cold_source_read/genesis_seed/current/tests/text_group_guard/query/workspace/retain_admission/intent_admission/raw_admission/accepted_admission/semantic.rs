//! Genuine opaque SDK/Core originals remain fixed while valid-MAC WORK is mutated.
use super::*;
#[test]
fn actual_first_b_rejects_fifteen_authenticated_semantic_mutations() {
    let f = accepted_fixture(0, 53000);
    let store = f.original.layout.checked_store().unwrap();
    for case in 0..15 {
        f.accepted
            .assert_original_b_semantic_mutation_for_test(store, case);
    }
}
