//! Genuine unused-lane conversion only. These tests claim no token miss, ordinary
//! accepted Create, signer admission, active worker drain or ordinary WD/WC retirement.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::{
    active_text_lane_count_for_test, PreparedV2TextCreateLane, PreparedV2TextLane,
    V2TextCreateLane, V2TextLane, V2_TEXT_CREATE_LANE_BYTES,
};
use crate::store::persistence::PreparedV2TextSourceObserver;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

fn claim_unused_lane(original: &TextFixture) -> V2TextLane {
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let prepared = PreparedV2TextSourceObserver::prepare(store, &owner).unwrap();
    let observer = store
        .with_v2_text_current(&owner, |a| prepared.capture(a))
        .unwrap();
    let mut prepared = PreparedV2TextLane::prepare(store, &owner, &observer).unwrap();
    let mut retained = None;
    store
        .with_v2_text_current(&owner, |a| {
            store
                .lock_v2_text_owner_namespace(a)?
                .claim_text_lane_into(&mut prepared, &mut retained)
        })
        .unwrap();
    prepared.drain_unused_outside_writers();
    retained.unwrap()
}

#[test]
fn actual_three_family_unused_lane_conversion_keeps_same_original_and_strong_exclusion() {
    for family in 0..3 {
        let original = text_fixture(family, 0x7ee00 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let mut raw = Some(claim_unused_lane(&original));
        let raw_observation = raw.as_ref().unwrap().observe_original();
        let mut prepared =
            PreparedV2TextCreateLane::prepare(store, &owner, raw.as_ref().unwrap()).unwrap();
        let capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let resources = capture.resources().clone();
        let mut retained: Option<V2TextCreateLane> = None;
        assert!(store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.convert_text_create_lane_into(&mut prepared, &mut raw, &mut retained)?;
                // The outer owner survives a lost callback return, with no raw Option hole.
                Err::<(), _>(critical_namespaces::refused())
            })
            .is_err());
        assert!(raw.is_none());
        assert!(retained.is_some());
        assert!(!raw_observation.is_cancelled());
        assert_eq!(active_text_lane_count_for_test(store), 1);
        let observation = retained.as_ref().unwrap().observe_original();
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.convert_text_create_lane_into(&mut prepared, &mut raw, &mut retained)
            })
            .unwrap();
        observation
            .require_same_original(retained.as_ref().unwrap())
            .unwrap();

        let observer = PreparedV2TextSourceObserver::prepare(store, &owner).unwrap();
        let observer = store
            .with_v2_text_current(&owner, |a| observer.capture(a))
            .unwrap();
        let mut another = PreparedV2TextLane::prepare(store, &owner, &observer).unwrap();
        let mut replacement = None;
        assert!(store
            .with_v2_text_current(&owner, |a| {
                store
                    .lock_v2_text_owner_namespace(a)?
                    .claim_text_lane_into(&mut another, &mut replacement)
            })
            .is_err());
        drop(retained);
        assert!(observation.is_cancelled());
        assert!(raw_observation.is_cancelled());
        assert_eq!(active_text_lane_count_for_test(store), 1);
        assert!(store
            .with_v2_text_current(&owner, |a| {
                store
                    .lock_v2_text_owner_namespace(a)?
                    .claim_text_lane_into(&mut another, &mut replacement)
            })
            .is_err());
        assert!(replacement.is_none(), "Drop is not ordinary retirement");
    }
}

#[test]
fn actual_unused_lane_conversion_preserves_outside_owner_after_late_scope_refusal() {
    let original = text_fixture(0, 0x7ee10);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut raw = Some(claim_unused_lane(&original));
    let mut prepared =
        PreparedV2TextCreateLane::prepare(store, &owner, raw.as_ref().unwrap()).unwrap();
    let capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let resources = capture.resources().clone();
    let mut retained = None;
    assert!(store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.convert_text_create_lane_into(&mut prepared, &mut raw, &mut retained)?;
            owner.warm_scope().unwrap().revocation().revoke();
            Ok(())
        })
        .is_err());
    assert!(raw.is_none());
    let observation = retained.as_ref().unwrap().observe_original();
    assert!(observation.is_cancelled());
    assert_eq!(active_text_lane_count_for_test(store), 1);
    assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
}

#[test]
fn actual_create_lane_metadata_keeps_its_independent_payment_through_last_negative_tail() {
    let original = text_fixture(0, 0x7ee20);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let mut raw = Some(claim_unused_lane(&original));
    let capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let resources = capture.resources().clone();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let before = pool.charged_for_test();
    let mut prepared =
        PreparedV2TextCreateLane::prepare(store, &owner, raw.as_ref().unwrap()).unwrap();
    assert_eq!(pool.charged_for_test(), before + V2_TEXT_CREATE_LANE_BYTES);
    let mut retained = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.convert_text_create_lane_into(&mut prepared, &mut raw, &mut retained)
        })
        .unwrap();
    let observation = retained.as_ref().unwrap().observe_original();
    drop(prepared);
    drop(retained);
    assert!(observation.is_cancelled());
    assert_eq!(pool.charged_for_test(), before + V2_TEXT_CREATE_LANE_BYTES);
    drop(observation);
    assert_eq!(pool.charged_for_test(), before);
    assert_eq!(active_text_lane_count_for_test(store), 1);
}
