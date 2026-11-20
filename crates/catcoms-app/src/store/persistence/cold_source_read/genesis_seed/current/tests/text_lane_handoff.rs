//! Genuine original Current lane transfer across lost callback return and late cancellation.
#![cfg(windows)]
use super::*;
use crate::store::owner_evidence::canonical::text_lane::{
    active_text_lane_count_for_test, PreparedV2TextLane,
};
use crate::store::persistence::{PreparedV2TextSourceObserver, V2TextOwnedCurrent};
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

#[test]
fn actual_text_claim_caches_same_unique_lane_before_lost_return_or_late_scope_refusal() {
    let mut f = fixture(0x7e8b0);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    let original_nonce = current.scope.nonce();
    let original_owner = current.scope.owner_identity();
    let source_id = current.data().source;
    let owned = V2TextOwnedCurrent::from_warm(current);
    let loan = owned.borrow_current().unwrap();
    let store = f.layout.checked_store().unwrap();
    let observer_preparation = PreparedV2TextSourceObserver::prepare(store, &loan).unwrap();
    let observer = store
        .with_v2_text_current(&loan, |a| observer_preparation.capture(a))
        .unwrap();
    let mut prepared = PreparedV2TextLane::prepare(store, &loan, &observer).unwrap();
    let charged = f.pool.charged_for_test();
    let mut retained = None;
    // Real slot transfer succeeds, then the owning caller loses the callback result. The
    // unique handle was already stored outside AD, not returned as a transient T.
    assert!(store
        .with_v2_text_current(&loan, |a| {
            let mut namespace = store.lock_v2_text_owner_namespace(a)?;
            namespace.claim_text_lane_into(&mut prepared, &mut retained)?;
            Err::<(), _>(critical_namespaces::refused())
        })
        .is_err());
    let original = retained.as_ref().unwrap().observe_original();
    assert!(!original.is_cancelled());
    assert_eq!(active_text_lane_count_for_test(store), 1);
    assert_eq!(f.pool.charged_for_test(), charged);
    store
        .with_v2_text_current(&loan, |a| {
            let mut namespace = store.lock_v2_text_owner_namespace(a)?;
            namespace.claim_text_lane_into(&mut prepared, &mut retained)
        })
        .unwrap();
    original
        .require_same_original(retained.as_ref().unwrap())
        .unwrap();
    assert_eq!(loan.source().data().source, source_id);
    assert_eq!(loan.warm_scope().unwrap().nonce(), original_nonce);
    assert!(Arc::ptr_eq(
        &original_owner,
        &loan.warm_scope().unwrap().owner_identity()
    ));

    // Now cancellation wins after the idempotent original claim but before the generic
    // Scope wrapper's final check. Its Err still cannot consume/drop the outside handle.
    assert!(store
        .with_v2_text_current(&loan, |a| {
            let mut namespace = store.lock_v2_text_owner_namespace(a)?;
            namespace.claim_text_lane_into(&mut prepared, &mut retained)?;
            loan.warm_scope().unwrap().revocation().revoke();
            Ok(())
        })
        .is_err());
    assert!(retained.is_some());
    assert!(original.is_cancelled());
    assert_eq!(active_text_lane_count_for_test(store), 1);
    assert_eq!(f.pool.charged_for_test(), charged);
    assert!(store.with_v2_text_current(&loan, |_| Ok(())).is_err());
}
