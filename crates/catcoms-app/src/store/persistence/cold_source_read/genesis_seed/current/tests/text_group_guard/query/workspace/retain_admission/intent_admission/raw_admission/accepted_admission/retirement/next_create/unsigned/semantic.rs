//! Valid outer frames alone cannot replace the original second-B association.
//! The actual fixture executes both Create/SDK/sign/selector chains; these changes
//! exercise only its original-aware DATA verifier, without fabricating a receipt.
use super::*;
#[test]
fn actual_second_b_rejects_fifteen_valid_outer_wal_mutations() {
    let number = 0x83d00;
    let mut t = next_saved_a_fixture(0, number);
    let query = accepted_miss(&t);
    let mut next = None;
    prepare_unsigned(&mut t, &query, &mut next).unwrap();
    let mut unsigned = None;
    PreparedV2NextUnsignedAppend::take_complete_into(
        t.transaction.as_ref().unwrap(),
        &mut next,
        &mut unsigned,
    )
    .unwrap();
    let accepted = second_b(&mut t, Some(query), unsigned, number);
    let store = t.original.layout.checked_store().unwrap();
    accepted.assert_completed_node_manifest_for_test(store);
    for case in 0..15 {
        accepted.assert_completed_b_semantic_mutation_for_test(store, case);
    }
}

#[test]
fn actual_completed_parent_accepted_query_refuses_short_credit_before_copy_then_retries() {
    let t = next_saved_a_fixture(0, 0x83d20);
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let files = super::super::physical::known_files(&target);
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let before = pool.charged_for_test();
    let required = PreparedV2TextAcceptedQuery::metadata_fee_bytes_for_test();
    let held_bytes = CAPACITY
        .checked_sub(before)
        .unwrap()
        .checked_sub(required - 1)
        .unwrap();
    let held = pool.try_reserve(held_bytes).unwrap();
    assert_eq!(CAPACITY - pool.charged_for_test(), required - 1);
    let original = t.transaction.as_ref().unwrap().observe_original();
    assert!(PreparedV2TextAcceptedQuery::prepare(
        store,
        owner.source(),
        t.transaction.as_ref().unwrap()
    )
    .is_err());
    original
        .require_same_original(&t.transaction.as_ref().unwrap().observe_original())
        .unwrap();
    assert_eq!(pool.charged_for_test(), before + held_bytes);
    assert_eq!(super::super::physical::known_files(&target), files);
    assert!(!t.lane.workspace_active_for_test());
    drop(held);
    assert_eq!(pool.charged_for_test(), before);
    let query = PreparedV2TextAcceptedQuery::prepare(
        store,
        owner.source(),
        t.transaction.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(pool.charged_for_test(), before + required);
    assert!(query.result_data().is_err());
    assert_eq!(super::super::physical::known_files(&target), files);
    drop(query);
    assert_eq!(pool.charged_for_test(), before);
}
