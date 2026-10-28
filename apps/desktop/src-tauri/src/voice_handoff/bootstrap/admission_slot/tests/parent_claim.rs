//! Actual Actor/physical native custody at the new inert ledger boundary. No
//! browser/Ready/RTC success is inferred from a native reservation.
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
    let native = route.session().native_hold().unwrap();
    (fee, native)
}

#[tokio::test]
async fn native_admission_parent_claim_waits_for_real_ledger_capacity_without_reminting_root() {
    for full_positions in [false, true] {
        let (f, context, route, _cleanup) = setup().await;
        let (_fee, native) = paid_native(&route);
        let ledger = f.state.voice_handoff.clone();
        let resources = route.selected().resources();
        let group = route.selected().binding().scope.group;
        let mut held = Vec::new();
        if full_positions {
            for generation in 10..42 {
                held.push(
                    ledger
                        .reserve_parent(
                            route.selected().clone(),
                            route.session().identity().clone(),
                            generation,
                        )
                        .unwrap(),
                );
            }
        }
        queue_requested(&context, &route, &native, 1).unwrap();
        until(|| {
            route
                .admission_work()
                .as_ref()
                .is_some_and(VoiceAdmissionWork::is_retired)
        })
        .await;
        // Refuse the later PreparedPhase contribution specifically: the first
        //2304B Row fee fits, but the real2KiB second payment cannot. Root's SAME
        // already-admitted outcome must survive this local-only refusal.
        let filler = if full_positions {
            None
        } else {
            Some(
                resources
                    .charge(
                        group,
                        GROUP_BYTES - resources.group_counts(group).unwrap().0 - 2304,
                        ResourceKind::Retained,
                    )
                    .unwrap(),
            )
        };
        let before = resources.counts();
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert_eq!(view(&route).phase, Phase::Staged);
        let original = view(&route).incarnation.unwrap();
        for _ in 0..3 {
            route
                .take_admission_finalization(&native)
                .unwrap()
                .unwrap()
                .finish(&context);
            assert_eq!(view(&route).phase, Phase::Staged);
            assert_eq!(view(&route).incarnation, Some(original));
        }
        // Registration was installed from the original result; retries add no
        // payment, mint, Actor command or second retained native candidate.
        assert_eq!(resources.counts(), before);
        drop(filler);
        drop(held.pop());
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert_eq!(view(&route).phase, Phase::Reserved);
        assert_eq!(view(&route).incarnation, Some(original));
        let first = {
            let state = route.row.state.lock().unwrap();
            let parent = state
                .admission_slot
                .parent
                .as_ref()
                .expect("actual native claim");
            assert!(parent.belongs_to(&ledger));
            assert!(parent.matches(route.selected(), route.session().identity(), 1));
            parent.fixture_id()
        };
        assert!(ledger.fixture_parent_reserved(first));
        cancel(&route, 1).unwrap();
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert_eq!(view(&route).phase, Phase::Returned);
        assert!(!ledger.fixture_parent_reserved(first));
        queue_requested(&context, &route, &native, 2).unwrap();
        finalize_original(&route, &context, &native).await;
        assert_eq!(view(&route).phase, Phase::Reserved);
        assert_eq!(view(&route).incarnation, Some(original));
        let second = route
            .row
            .state
            .lock()
            .unwrap()
            .admission_slot
            .parent
            .as_ref()
            .unwrap()
            .fixture_id();
        assert!(second.logical()[8..] > first.logical()[8..]);
        route.close_admission_slot();
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert!(!ledger.fixture_parent_reserved(second));
        drop(held);
        drop(native);
        f.close().await;
    }
}

#[tokio::test]
async fn native_admission_parent_claim_closing_joins_actual_ledger_stamp_return() {
    let (f, context, route, _cleanup) = setup().await;
    let (_fee, native) = paid_native(&route);
    queue_requested(&context, &route, &native, 1).unwrap();
    finalize_original(&route, &context, &native).await;
    let (entered, at_return) = std::sync::mpsc::sync_channel(0);
    let (release, released) = std::sync::mpsc::channel();
    let id = {
        let mut state = route.row.state.lock().unwrap();
        let parent = state.admission_slot.parent.as_mut().unwrap();
        let id = parent.fixture_id();
        parent.fixture_hold_return(entered, released);
        id
    };
    let before = route.selected().resources().counts();
    cancel(&route, 1).unwrap();
    let finalization = route.take_admission_finalization(&native).unwrap().unwrap();
    let (reached, snapshot, next, occupied, during) = std::thread::scope(|scope| {
        let context = &context;
        let worker = scope.spawn(move || finalization.finish(context));
        let reached = at_return.recv_timeout(std::time::Duration::from_secs(5));
        let snapshot = capacity(&route, context, CapacityKind::Observe, 1);
        let next = capacity(&route, context, CapacityKind::Register, 2);
        let occupied = f.state.voice_handoff.fixture_parent_reserved(id);
        let during = route.selected().resources().counts();
        let _ = release.send(());
        worker.join().unwrap();
        (reached, snapshot, next, occupied, during)
    });
    reached.unwrap();
    assert_eq!(snapshot.unwrap().state, CapacityState::Closing);
    assert!(matches!(next, Err(Refused::Bounds)));
    assert!(
        occupied,
        "the real ledger stamp outlives both fee destructors"
    );
    // Finalization destroys its Root capacity before reclaiming Slot's parent.
    // Those independently paid owners may already have returned too. The unit
    // barrier pins this parent's exact4352B; this boundary must not claim an
    // aggregate Actor/Root refund from native per-generation return.
    assert!(during.0 <= before.0 - 2304 - 2048);
    assert_eq!(view(&route).phase, Phase::Returned);
    assert!(!f.state.voice_handoff.fixture_parent_reserved(id));
    route.close_admission_slot();
    route
        .take_admission_finalization(&native)
        .unwrap()
        .unwrap()
        .finish(&context);
    drop(native);
    f.close().await;
}

#[tokio::test]
async fn native_admission_parent_claim_final_fence_rejects_foreign_ledger_session_or_generation() {
    for changed in 0..3 {
        let (f, context, route, _cleanup) = setup().await;
        let (_fee, native) = paid_native(&route);
        queue_requested(&context, &route, &native, 1).unwrap();
        until(|| {
            route
                .admission_work()
                .as_ref()
                .is_some_and(VoiceAdmissionWork::is_retired)
        })
        .await;
        let ui = f.state.ui_session_commit.clone().lock_owned().await;
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert_eq!(view(&route).phase, Phase::Staged);
        let ledger = if changed == 0 {
            Arc::new(crate::voice_handoff::ledger::Ledger::default())
        } else {
            f.state.voice_handoff.clone()
        };
        let session = if changed == 1 {
            Arc::new(SessionIdentity(route.session().identity().0))
        } else {
            route.session().identity().clone()
        };
        let foreign = ledger
            .reserve_parent(
                route.selected().clone(),
                session,
                if changed == 2 { 2 } else { 1 },
            )
            .unwrap();
        let foreign_id = foreign.fixture_id();
        let original = {
            let mut state = route.row.state.lock().unwrap();
            let Outcome::Candidate(_, parent) = state
                .admission_slot
                .invocation
                .as_mut()
                .unwrap()
                .outcome
                .as_mut()
                .unwrap()
            else {
                panic!("actual retained candidate")
            };
            parent.replace(foreign)
        };
        drop(original); // destruction never occurs under State
        drop(ui);
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        assert_eq!(view(&route).phase, Phase::Closed);
        assert!(!ledger.fixture_parent_reserved(foreign_id));
        assert!(route.row.state.lock().unwrap().admission_slot.collected());
        drop(native);
        f.close().await;
    }
}
