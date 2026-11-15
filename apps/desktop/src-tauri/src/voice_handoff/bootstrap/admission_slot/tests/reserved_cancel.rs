//! Operation30 cancellation interleavings at the actual Root/native boundary.
//! The synchronous paid native caller deliberately runs no background worker:
//! each test chooses whether real original-owner collection precedes the reply.
use super::*;

fn paid_native(route: &Route) -> (Credit, crate::voice_handoff::channel::NativeHold) {
    let fee = route
        .selected()
        .resources()
        .charge(
            route.selected().binding().scope.group,
            NATIVE_TASK_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    (fee, route.session().native_hold().unwrap())
}

#[tokio::test]
async fn capacity_reserved_cancel_preserves_registration_before_and_after_actual_return() {
    for return_before_snapshot in [false, true] {
        let (f, context, route, _cleanup) = setup().await;
        let (_fee, native) = paid_native(&route);
        queue_requested(&context, &route, &native, 1).unwrap();
        finalize_original(&route, &context, &native).await;
        let original = view(&route).incarnation.unwrap();
        assert_eq!(view(&route).phase, Phase::Reserved);
        let request = CapacityRequest {
            kind: CapacityKind::Cancel,
            generation: 1,
        };
        let cancelled = if return_before_snapshot {
            // SAME production request admission, Root negative floor and notify.
            // Execute the actual affine finisher at the possible worker handoff
            // before the public adapter's real snapshot stage. No status is set
            // by this test and no replacement Root call is made.
            route.apply_capacity(&context, request).unwrap();
            route
                .take_admission_finalization(&native)
                .unwrap()
                .unwrap()
                .finish(&context);
            route.capacity_snapshot(1).unwrap()
        } else {
            // Exercise the complete real public handler with collection unable
            // to preempt it. This failed before the currency/negative distinction:
            // Cancel made RootHighPreclaim noncurrent and snapshot closed Slot.
            capacity(&route, &context, CapacityKind::Cancel, 1).unwrap()
        };
        assert_eq!(
            cancelled.state,
            if return_before_snapshot {
                CapacityState::Returned
            } else {
                CapacityState::Closing
            }
        );
        assert_eq!(cancelled.incarnation, Some(original));
        assert!(!route.row.state.lock().unwrap().admission_slot.closed);
        if !return_before_snapshot {
            route
                .take_admission_finalization(&native)
                .unwrap()
                .unwrap()
                .finish(&context);
        }
        let returned = capacity(&route, &context, CapacityKind::Observe, 1).unwrap();
        assert_eq!(returned.state, CapacityState::Returned);
        assert_eq!(returned.incarnation, Some(original));
        // Exact duplicate Cancel after return also cannot turn a round fence
        // into whole-Session closure or debit another generation/Root mint.
        assert_eq!(
            capacity(&route, &context, CapacityKind::Cancel, 1)
                .unwrap()
                .state,
            CapacityState::Returned
        );
        queue_requested(&context, &route, &native, 2).unwrap();
        finalize_original(&route, &context, &native).await;
        let current = capacity(&route, &context, CapacityKind::Observe, 2).unwrap();
        assert_eq!(current.state, CapacityState::Reserved);
        assert_eq!(current.incarnation, Some(original));
        let before = route.selected().resources().counts();
        let stale = capacity(&route, &context, CapacityKind::Cancel, 1).unwrap();
        assert_eq!(stale.state, CapacityState::Superseded);
        assert_eq!(stale.incarnation, Some(original));
        assert_eq!(
            capacity(&route, &context, CapacityKind::Observe, 2)
                .unwrap()
                .state,
            CapacityState::Reserved
        );
        assert_eq!(route.selected().resources().counts(), before);
        assert!(!route.row.state.lock().unwrap().admission_slot.closed);
        route.close_admission_slot();
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        drop(native);
        f.close().await;
    }
}

#[tokio::test]
async fn capacity_reserved_cancel_cannot_mask_a_revoked_call_or_original_registration() {
    for revoke_registration in [false, true] {
        let (f, context, route, _cleanup) = setup().await;
        let (_fee, native) = paid_native(&route);
        queue_requested(&context, &route, &native, 1).unwrap();
        finalize_original(&route, &context, &native).await;
        if revoke_registration {
            route
                .row
                .state
                .lock()
                .unwrap()
                .admission_slot
                .registration
                .as_ref()
                .unwrap()
                .close();
        } else {
            route.selected().lease().revoke();
        }
        let reply = capacity(&route, &context, CapacityKind::Cancel, 1).unwrap();
        assert_eq!(reply.state, CapacityState::Closing);
        assert!(route.row.state.lock().unwrap().admission_slot.closed);
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert_eq!(
            capacity(&route, &context, CapacityKind::Observe, 1)
                .unwrap()
                .state,
            CapacityState::Closed
        );
        assert!(matches!(
            capacity(&route, &context, CapacityKind::Register, 2),
            Err(Refused::Bounds)
        ));
        drop(native);
        f.close().await;
    }
}
