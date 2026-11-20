//! Actual domain provenance and paid tails. These first-A fixtures do not represent
//! a same-group remount or a blocked accepted-publication callback.
use super::*;

#[test]
fn actual_same_source_equal_genesis_domains_cannot_exchange_bound_founder_anchor() {
    let (f, selected) = saved_original_fixture(0x7f910);
    PreparedV2UnsignedOriginalCustody::assert_other_live_domain_rejected_for_test(
        f.original.layout.checked_store().unwrap(),
        &f.original.owner,
        selected.selected_data().unwrap(),
        &f.transaction,
    );
}

#[test]
fn actual_foreign_source_observer_and_anchor_refuse_without_replacing_inputs() {
    let (a, selected_a) = saved_original_fixture(0x7f920);
    let (b, selected_b) = saved_original_fixture(0x7f930);
    PreparedV2UnsignedOriginalCustody::assert_other_real_source_rejected_for_test(
        (
            a.original.layout.checked_store().unwrap(),
            &a.original.owner,
            selected_a.selected_data().unwrap(),
            &a.transaction,
        ),
        (
            b.original.layout.checked_store().unwrap(),
            &b.original.owner,
            selected_b.selected_data().unwrap(),
            &b.transaction,
        ),
    );
}

#[test]
fn actual_core_verifier_keeps_exact_domain_fee_after_root_domain_and_sdk_drop() {
    let (f, selected) = saved_original_fixture(0x7f940);
    PreparedV2UnsignedOriginalCustody::assert_actual_core_tail_keeps_domain_fee_for_test(
        f.original.layout.checked_store().unwrap(),
        &f.original.owner,
        selected.selected_data().unwrap(),
        &f.transaction,
    );
}

#[test]
fn explicit_actual_domain_close_preserves_data_but_refuses_positive_use_with_source_live() {
    let (f, selected) = saved_original_fixture(0x7f950);
    let store = f.original.layout.checked_store().unwrap();
    let domain_dropped = Arc::new(AtomicBool::new(false));
    let model_dropped = Arc::new(AtomicBool::new(false));
    let sdk_dropped = Arc::new(AtomicBool::new(false));
    let custody = PreparedV2UnsignedOriginalCustody::prepare_with_domain_fee_notice_for_test(
        store,
        &f.original.owner,
        selected.selected_data().unwrap(),
        &f.transaction,
        (
            domain_dropped.clone(),
            model_dropped.clone(),
            sdk_dropped.clone(),
        ),
    )
    .unwrap();
    let digest = unsigned_digest(&custody, &f.transaction);
    let original = custody.observe_original();
    let same = original.clone();
    original.validate_negative().unwrap();
    custody.assert_explicit_domain_close_for_test(store, &f.original.owner, &f.transaction);
    assert!(original.validate_negative().is_err());
    assert_eq!(unsigned_digest(&custody, &f.transaction), digest);
    original.require_same_original(&same).unwrap();
    for flag in [&domain_dropped, &model_dropped, &sdk_dropped] {
        assert!(!flag.load(Ordering::SeqCst));
    }
    drop(custody);
    assert!(model_dropped.load(Ordering::SeqCst));
    assert!(sdk_dropped.load(Ordering::SeqCst));
    // The tiny original observation retains ONLY the actual domain16KiB and its
    // separate transaction metadata, not the resident model1MiB or SDK2MiB.
    assert!(!domain_dropped.load(Ordering::SeqCst));
    assert!(original.validate_negative().is_err());
    original.require_same_original(&same).unwrap();
    drop(original);
    assert!(!domain_dropped.load(Ordering::SeqCst));
    drop(same);
    assert!(domain_dropped.load(Ordering::SeqCst));
    let current = f.original.owner.borrow_current().unwrap();
    store.with_v2_text_current(&current, |_| Ok(())).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
}
