//! Actual settled Current -> cached text Scope/kernel -> original negative projection source.
//! These tests issue no fabricated selected O, transaction, Saved receipt or positive token.
//! Root composes this child only with the actual host/Scope/kernel consumers.
#![cfg(windows)]
use super::*;
use crate::store::persistence::{
    PreparedV2TextSourceObserver, V2TextOwnedCurrent, V2_TEXT_SOURCE_OBSERVER_BYTES,
};
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn actual_text_owner_requires_settlement_and_origin_does_not_retain_original_read_capture() {
    let mut f = fixture(0x7e870);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    // A real saved Current still has its original activation seed; no scalar missing-file
    // check or writer observation turns this into the operational settled prerequisite.
    assert!(current.text_owner_ref().is_err());
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    let read_dropped = Arc::new(AtomicBool::new(false));
    Arc::get_mut(
        &mut Arc::get_mut(&mut current.resources.source.capture)
            .expect("genuine original capture uniquely owned")
            .fee,
    )
    .expect("genuine original read debit uniquely owned")
    .observe_drop_for_test(read_dropped.clone());
    let original_source = current.data().source;
    let original_nonce = current.scope.nonce();
    let original_owner = current.scope.owner_identity();
    let original_writer = current.writer.observe_original();
    let sdk_entries = super::super::super::SDK_ENTRIES.with(|n| n.get());
    let owned = V2TextOwnedCurrent::from_warm(current);
    let store = f.layout.checked_store().unwrap();
    let loan = owned.borrow_current().unwrap();
    assert_eq!(loan.source().data().source, original_source);
    assert_eq!(loan.warm_scope().unwrap().nonce(), original_nonce);
    assert!(Arc::ptr_eq(
        &original_owner,
        &loan.warm_scope().unwrap().owner_identity()
    ));
    loan.require_original_negative(store, &original_writer)
        .unwrap();
    let baseline = f.pool.charged_for_test();
    let mut preparation = PreparedV2TextSourceObserver::prepare(store, &loan).unwrap();
    let metadata_dropped = Arc::new(AtomicBool::new(false));
    preparation.observe_metadata_drop_for_test(metadata_dropped.clone());
    assert_eq!(
        f.pool.charged_for_test(),
        baseline + V2_TEXT_SOURCE_OBSERVER_BYTES
    );
    let observer = store
        .with_v2_text_current(&loan, |admission| {
            let namespace = store.lock_v2_text_owner_namespace(admission)?;
            namespace.require_installed_text_writer(admission.writer_observation()?)?;
            preparation.capture(admission)
        })
        .unwrap();
    assert!(observer.is_live());
    let core_negative = observer.origin_negative();
    assert!(core_negative.is_live());
    assert_eq!(
        super::super::super::SDK_ENTRIES.with(|n| n.get()),
        sdk_entries
    );
    drop(preparation);
    // Drop actual source/node while the Core-facing original observer still lives. The
    // SPECIFIC source invoice must drain; aggregation of unrelated refunds cannot pass.
    drop(loan);
    drop(owned);
    assert!(read_dropped.load(Ordering::SeqCst));
    assert!(!core_negative.is_live());
    assert!(!metadata_dropped.load(Ordering::SeqCst));
    drop(observer);
    assert!(!metadata_dropped.load(Ordering::SeqCst));
    drop(core_negative);
    assert!(metadata_dropped.load(Ordering::SeqCst));
}

#[test]
fn actual_scope_writer_or_task_revocation_closes_original_text_origin_while_node_still_lives() {
    for cut in 0..3 {
        let mut f = fixture(0x7e880 + cut);
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
        current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .unwrap();
        let store = f.layout.checked_store().unwrap();
        let loan = current.text_owner_ref().unwrap();
        drop(loan);
        let owned = V2TextOwnedCurrent::from_warm(current);
        let loan = owned.borrow_current().unwrap();
        let preparation = PreparedV2TextSourceObserver::prepare(store, &loan).unwrap();
        let observer = store
            .with_v2_text_current(&loan, |admission| {
                let namespace = store.lock_v2_text_owner_namespace(admission)?;
                namespace.require_installed_text_writer(admission.writer_observation()?)?;
                preparation.capture(admission)
            })
            .unwrap();
        let source_id = loan.source().data().source;
        assert!(observer.is_live());
        match cut {
            0 => loan.warm_scope().unwrap().revocation().revoke(),
            1 => owned.cancel_warm_writer_for_test(),
            _ => loan.task().cancel_original(),
        }
        assert!(!observer.is_live(), "cut {cut}");
        let callbacks = std::cell::Cell::new(0);
        assert!(store
            .with_v2_text_current(&loan, |_| {
                callbacks.set(callbacks.get() + 1);
                Ok(())
            })
            .is_err());
        assert_eq!(callbacks.get(), 0);
        assert_eq!(loan.source().data().source, source_id);
    }
}

#[test]
fn late_text_scope_refusal_unlocks_admission_before_returned_resource_and_fee_drop() {
    struct ReturnedResource<'store> {
        store: &'store ServerStore,
        dropped: Arc<AtomicBool>,
        _fee: CatalogReadReservation,
    }
    impl Drop for ReturnedResource<'_> {
        fn drop(&mut self) {
            assert!(
                self.store.chat_mount.writer_available_for_test(),
                "owning callback outcome must drain after the actual AD writer"
            );
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    let mut f = fixture(0x7e890);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    let owned = V2TextOwnedCurrent::from_warm(current);
    let loan = owned.borrow_current().unwrap();
    let store = f.layout.checked_store().unwrap();
    let before = f.pool.charged_for_test();
    let dropped = Arc::new(AtomicBool::new(false));
    let mut resource = Some(ReturnedResource {
        store,
        dropped: dropped.clone(),
        _fee: f.pool.try_reserve(4096).unwrap(),
    });
    assert_eq!(f.pool.charged_for_test(), before + 4096);
    let result = store.with_v2_text_current(&loan, |admission| {
        let namespace = store.lock_v2_text_owner_namespace(admission)?;
        namespace.require_installed_text_writer(admission.writer_observation()?)?;
        loan.warm_scope().unwrap().revocation().revoke();
        // The callback borrows the outer owning slot across every fallible operation.
        // Only an infallible transfer remains after taking its actual paid resource.
        Ok(resource.take().expect("same original outside-writer slot"))
    });
    assert!(result.is_err());
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(f.pool.charged_for_test(), before);
}

#[test]
fn actual_negative_text_lane_keeps_original_backing_and_strong_debt_after_caller_drop() {
    use crate::store::owner_evidence::canonical::text_lane::{
        active_text_lane_count_for_test, PreparedV2TextLane, V2_TEXT_LANE_BUFFER_BYTES,
        V2_TEXT_LANE_ENTRY_BYTES,
    };
    let mut f = fixture(0x7e8a0);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    let owned = V2TextOwnedCurrent::from_warm(current);
    let loan = owned.borrow_current().unwrap();
    let store = f.layout.checked_store().unwrap();
    let observer_preparation = PreparedV2TextSourceObserver::prepare(store, &loan).unwrap();
    let observer = store
        .with_v2_text_current(&loan, |a| observer_preparation.capture(a))
        .unwrap();
    assert_eq!(active_text_lane_count_for_test(store), 0);
    let before = f.pool.charged_for_test();
    let mut first = PreparedV2TextLane::prepare(store, &loan, &observer).unwrap();
    let mut competing = PreparedV2TextLane::prepare(store, &loan, &observer).unwrap();
    let paid = before + 2 * (V2_TEXT_LANE_ENTRY_BYTES + V2_TEXT_LANE_BUFFER_BYTES);
    assert_eq!(f.pool.charged_for_test(), paid);
    first.drain_unused_outside_writers();
    competing.drain_unused_outside_writers();
    assert_eq!(
        f.pool.charged_for_test(),
        paid,
        "unstarted admission still needs its backing"
    );
    let head_path = loan.source().source().capture.target.join("head.bin");
    let head = std::fs::read(&head_path).unwrap();
    let scope = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
    // Borrow an OUTSIDE retained slot through every fallible callback; no owning Lane
    // can be destroyed under AD if the helper's final original-source postcheck refuses.
    let mut retained = None;
    store
        .with_v2_text_current(&loan, |a| {
            let mut namespace = store.lock_v2_text_owner_namespace(a)?;
            namespace.claim_text_lane_into(&mut first, &mut retained)?;
            Ok(())
        })
        .unwrap();
    let lane = retained.take().unwrap();
    first.drain_unused_outside_writers();
    assert_eq!(
        f.pool.charged_for_test(),
        paid,
        "first backing now belongs to Runtime"
    );
    assert_eq!(active_text_lane_count_for_test(store), 1);
    store
        .with_v2_text_current(&loan, |a| {
            let mut namespace = store.lock_v2_text_owner_namespace(a)?;
            lane.require_registered(&mut namespace)
        })
        .unwrap();
    let original = lane.observe_original();
    assert!(!original.is_cancelled());
    original.require_same_original(&lane).unwrap();
    drop(lane);
    assert!(original.is_cancelled());
    assert!(
        observer.is_live(),
        "lane caller loss is not a new source/scope revocation"
    );
    assert_eq!(active_text_lane_count_for_test(store), 1);
    assert_eq!(
        f.pool.charged_for_test(),
        paid,
        "cancelled original registry debt remains paid"
    );
    let mut competing_slot = None;
    assert!(store
        .with_v2_text_current(&loan, |a| {
            let mut namespace = store.lock_v2_text_owner_namespace(a)?;
            namespace.claim_text_lane_into(&mut competing, &mut competing_slot)
        })
        .is_err());
    assert_eq!(active_text_lane_count_for_test(store), 1);
    assert_eq!(f.pool.charged_for_test(), paid);
    assert_eq!(std::fs::read(head_path).unwrap(), head);
    assert_eq!(
        std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
        scope
    );
}
