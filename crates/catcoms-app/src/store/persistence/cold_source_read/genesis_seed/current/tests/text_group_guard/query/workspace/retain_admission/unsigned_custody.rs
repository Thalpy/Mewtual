//! Genuine whole unsigned SDK/model custody. Scope loss invalidates positive use,
//! while immutable original DATA and its exact paid ownership remain recoverable.
use super::*;
use crate::store::persistence::PreparedV2UnsignedOriginalCustody;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn saved_original_fixture(number: u64) -> (RetainFixture, PreparedV2TextParentCapture) {
    let (mut f, selected_genesis) = retain_fixture_with_selected(0, number);
    let stage = stage_admission(&f);
    f.transaction.run_stage(stage).unwrap();
    let commit = commit_admission(&f);
    f.transaction.run_commit(commit).unwrap();
    (f, selected_genesis)
}

fn unsigned_digest(
    custody: &PreparedV2UnsignedOriginalCustody,
    original: &PreparedV2TextRetainTransaction,
) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let data = custody.unsigned_data(original).unwrap();
    (
        data.intent,
        *blake3::hash(data.signing_bytes).as_bytes(),
        *blake3::hash(data.image).as_bytes(),
    )
}

#[test]
fn actual_unsigned_custody_keeps_exact_fees_across_scope_stop_but_tiny_observer_does_not_pin_them()
{
    let (f, selected_genesis) = saved_original_fixture(0x7f8a0);
    let store = f.original.layout.checked_store().unwrap();
    let model_dropped = Arc::new(AtomicBool::new(false));
    let sdk_dropped = Arc::new(AtomicBool::new(false));
    let custody = PreparedV2UnsignedOriginalCustody::prepare_for_test(
        store,
        &f.original.owner,
        selected_genesis.selected_data().unwrap(),
        &f.transaction,
        model_dropped.clone(),
        sdk_dropped.clone(),
    )
    .unwrap();
    assert!(!model_dropped.load(Ordering::SeqCst));
    assert!(!sdk_dropped.load(Ordering::SeqCst));
    let original = custody.observe_original();
    let same = original.clone();
    original.require_same_original(&same).unwrap();
    let digest = unsigned_digest(&custody, &f.transaction);
    let owner = f.original.owner.borrow_current().unwrap();
    custody
        .validate_original(store, owner.source(), &f.transaction)
        .unwrap();
    let head = owner.source().source().capture.target.join("head.bin");
    let before = std::fs::read(&head).unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    assert!(custody
        .validate_original(store, owner.source(), &f.transaction)
        .is_err());
    // This is the SAME cached original DATA, not a new SDK derivation, observer,
    // signer permit or live Current. No mutable SDK/model loan exists in this API.
    assert_eq!(unsigned_digest(&custody, &f.transaction), digest);
    custody.require_original_plan_data(&f.transaction).unwrap();
    original.require_same_original(&same).unwrap();
    assert!(!model_dropped.load(Ordering::SeqCst));
    assert!(!sdk_dropped.load(Ordering::SeqCst));
    assert_eq!(std::fs::read(head).unwrap(), before);
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
    drop(custody);
    // Each notice belongs to its REAL original resident1M or SDK2M reservation;
    // the still-live tiny observation retains only its separate A metadata/source.
    assert!(model_dropped.load(Ordering::SeqCst));
    assert!(sdk_dropped.load(Ordering::SeqCst));
    original.require_same_original(&same).unwrap();
    assert!(f.transaction.saved_retained_action().is_ok());
}

#[test]
fn actual_unsigned_custody_refuses_other_real_a_owner_and_plan_without_replacing_original() {
    let (a, original_a) = saved_original_fixture(0x7f8b0);
    let (b, original_b) = saved_original_fixture(0x7f8c0);
    let store_a = a.original.layout.checked_store().unwrap();
    let store_b = b.original.layout.checked_store().unwrap();
    let model_a = Arc::new(AtomicBool::new(false));
    let sdk_a = Arc::new(AtomicBool::new(false));
    let model_b = Arc::new(AtomicBool::new(false));
    let sdk_b = Arc::new(AtomicBool::new(false));
    let custody_a = PreparedV2UnsignedOriginalCustody::prepare_for_test(
        store_a,
        &a.original.owner,
        original_a.selected_data().unwrap(),
        &a.transaction,
        model_a.clone(),
        sdk_a.clone(),
    )
    .unwrap();
    let custody_b = PreparedV2UnsignedOriginalCustody::prepare_for_test(
        store_b,
        &b.original.owner,
        original_b.selected_data().unwrap(),
        &b.transaction,
        model_b.clone(),
        sdk_b.clone(),
    )
    .unwrap();
    let digest_a = unsigned_digest(&custody_a, &a.transaction);
    let digest_b = unsigned_digest(&custody_b, &b.transaction);
    let plan_a = custody_a.observe_original();
    let plan_b = custody_b.observe_original();
    assert!(plan_a.require_same_original(&plan_b).is_err());
    assert!(plan_b.require_same_original(&plan_a).is_err());
    assert!(custody_a
        .require_original_plan_data(&b.transaction)
        .is_err());
    assert!(custody_b
        .require_original_plan_data(&a.transaction)
        .is_err());
    assert!(custody_a.unsigned_data(&b.transaction).is_err());
    assert!(custody_b.unsigned_data(&a.transaction).is_err());
    let owner_a = a.original.owner.borrow_current().unwrap();
    let owner_b = b.original.owner.borrow_current().unwrap();
    assert!(custody_a
        .validate_original(store_b, owner_b.source(), &b.transaction)
        .is_err());
    assert!(custody_a
        .validate_original(store_b, owner_b.source(), &a.transaction)
        .is_err());
    // Refusal is non-destructive. Both original opaque SDK plans and exact cached
    // bytes still match their OWN real A; no wrong owner/plan is adopted on retry.
    custody_a
        .validate_original(store_a, owner_a.source(), &a.transaction)
        .unwrap();
    custody_b
        .validate_original(store_b, owner_b.source(), &b.transaction)
        .unwrap();
    assert_eq!(unsigned_digest(&custody_a, &a.transaction), digest_a);
    assert_eq!(unsigned_digest(&custody_b, &b.transaction), digest_b);
    for notice in [&model_a, &sdk_a, &model_b, &sdk_b] {
        assert!(!notice.load(Ordering::SeqCst));
    }
    drop(custody_a);
    assert!(model_a.load(Ordering::SeqCst) && sdk_a.load(Ordering::SeqCst));
    assert!(!model_b.load(Ordering::SeqCst) && !sdk_b.load(Ordering::SeqCst));
    drop(custody_b);
    assert!(model_b.load(Ordering::SeqCst) && sdk_b.load(Ordering::SeqCst));
    assert!(plan_a.require_same_original(&plan_b).is_err());
}

#[test]
fn actual_unsigned_handoff_rejects_another_genuine_sdk_plan_and_preserves_owning_input() {
    let (f, selected_genesis) = saved_original_fixture(0x7f8d0);
    let store = f.original.layout.checked_store().unwrap();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let owner = f.original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = std::fs::read(target.join("text.wal")).unwrap();
    let charged = pool.charged_for_test();
    drop(owner);
    // The narrow Root fixture adapter derives TWO real SDK plans for this SAME A,
    // swaps only inside the otherwise private owning family, checks actual constructor
    // refusal/input preservation, then restores the SAME original and succeeds. It
    // exposes neither SDK/model/slot nor a way to manufacture an opaque observation.
    PreparedV2UnsignedOriginalCustody::assert_replaced_plan_rejected_for_test(
        store,
        &f.original.owner,
        selected_genesis.selected_data().unwrap(),
        &f.transaction,
    );
    assert_eq!(pool.charged_for_test(), charged);
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    assert!(f.transaction.saved_retained_action().is_ok());
}

mod domain;
