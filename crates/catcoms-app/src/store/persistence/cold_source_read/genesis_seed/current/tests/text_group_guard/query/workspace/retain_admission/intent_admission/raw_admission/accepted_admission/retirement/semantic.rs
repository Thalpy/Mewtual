//! Actual own B after real Core/finalization; no fake WD/WC receipt or drain.
use super::*;
#[test]
fn authenticated_supported_retirement_variants_bind_the_exact_original_first_b() {
    let mut f = accepted_fixture(0, 0x82100);
    select_and_finalize(&mut f);
    let before = std::fs::read(b_target(&f).join("text.wal")).unwrap();
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    f.accepted
        .prepare_retirement_data(store, owner.source())
        .unwrap();
    f.accepted.assert_retirement_semantic_mutations_for_test(
        store,
        &mut ChaCha20Rng::seed_from_u64(0x82101),
    );
    assert!(f.accepted.saved_retirement_deletion().is_err());
    assert!(f.accepted.ready_retirement_completion().is_err());
    assert!(f.accepted.saved_retirement_completion().is_err());
    assert_eq!(
        std::fs::read(b_target(&f).join("text.wal")).unwrap(),
        before
    );
    for path in f.accepted.original_retired_paths_for_test().unwrap() {
        assert!(path.is_file());
    }
}
