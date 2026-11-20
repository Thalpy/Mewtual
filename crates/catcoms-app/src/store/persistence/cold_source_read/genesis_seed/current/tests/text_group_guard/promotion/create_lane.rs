//! A real promotion binding permanently excludes conversion, even after the actual
//! admitted marker drained without starting IO. Idle is not the same as unused.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::PreparedV2TextCreateLane;

#[test]
fn actual_promotion_binding_refuses_ordinary_conversion_before_and_after_real_end_drain() {
    let f = promotion_fixture(0, 0x7ee30);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let mut raw = Some(f.lane);
    let mut create =
        PreparedV2TextCreateLane::prepare(store, &owner, raw.as_ref().unwrap()).unwrap();
    let resources = f.prepared.resources().clone();
    let mut lease =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_promotion_stage_into(
                raw.as_ref().unwrap(),
                &f.prepared,
                &mut lease,
                &mut admitted,
            )
        })
        .unwrap();
    let original = raw.as_ref().unwrap().observe_original();
    let mut converted = None;
    for active in [true, false] {
        if !active {
            // Actual affine work-end Drop, not a test boolean passed into admission.
            drop(admitted.take());
        }
        assert!(store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                guard.convert_text_create_lane_into(&mut create, &mut raw, &mut converted)
            })
            .is_err());
        assert!(converted.is_none());
        original
            .require_same_original(raw.as_ref().unwrap())
            .unwrap();
        assert_eq!(active_text_lane_count_for_test(store), 1);
    }
}
