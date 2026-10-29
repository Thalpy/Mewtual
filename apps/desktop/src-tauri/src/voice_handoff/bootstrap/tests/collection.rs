//! Collection must observe actual independent owners, not infer drainage from the UI, a
//! Row reference count, a timeout or an already-returned close reply.
use super::*;
use crate::voice_handoff::channel::{BOOTSTRAP_BYTES, BROWSER_CONTROL_BYTES, CONTROL_BYTES};

#[tokio::test]
async fn header_and_public_output_independently_keep_original_capacity_and_slot_fence() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let output = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let slot = output.public().slot;
    let route = f.state.voice_channels.route(slot).unwrap();
    let session = route.session().downgrade();
    let public = output.downgrade();
    let header = route.session().header().unwrap();
    drop(route); // Header owns Session directly, with no Row reference left.
    f.state.voice_authority.lock().unwrap().end(&owner);
    let before = selected.resources().counts();
    assert!(matches!(
        collect_channel(&f.state, &owner, [9; 32]).await.unwrap(),
        Status::Closed {
            native_setup_drained: true
        }
    ));
    assert!(
        f.state.voice_channels.route(slot).is_err(),
        "new routing is fenced before detaching original owners"
    );
    assert_eq!(
        selected.resources().counts(),
        (
            before.0 - super::super::super::channel::bootstrap::SETUP_WORK_BYTES,
            1
        )
    );
    assert_eq!(
        session.strong_count(),
        1,
        "only the actual header keeps Session alive"
    );
    assert_eq!(
        public.strong_count(),
        1,
        "the returned output is independent of Row"
    );
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows[usize::from(slot - 1)]
    .is_some());
    drop(header);
    assert_eq!(session.strong_count(), 0);
    let without_session = selected.resources().counts();
    assert_eq!(
        without_session,
        (
            before.0
                - super::super::super::channel::bootstrap::SETUP_WORK_BYTES
                - CONTROL_BYTES
                - BROWSER_CONTROL_BYTES,
            0
        )
    );
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert!(
        f.state
            .voice_channels
            .index
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows[usize::from(slot - 1)]
        .is_some(),
        "a paid returned public output still blocks slot reuse"
    );
    drop(output);
    assert_eq!(public.strong_count(), 0);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(Option::is_none));
    assert_eq!(
        selected.resources().counts(),
        without_session,
        "the test's explicit weak allocation owners retain their SAME payments"
    );
    drop(public);
    assert_eq!(
        selected.resources().counts(),
        (without_session.0 - ROW_BYTES, 0)
    );
    drop(session);
    assert_eq!(
        selected.resources().counts(),
        (without_session.0 - ROW_BYTES - BOOTSTRAP_BYTES, 0)
    );
    let final_counts = selected.resources().counts();
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert_eq!(
        selected.resources().counts(),
        final_counts,
        "a lost collection reply remains exact no-work history"
    );
    f.close().await;
}

#[tokio::test]
async fn live_owner_keeps_closed_fence_and_wrong_executor_cannot_detach_original_session() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let output = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let slot = output.public().slot;
    let before = selected.resources().counts();
    assert!(matches!(
        collect_channel(&f.state, &owner, [8; 32]).await,
        Err(Refused::Binding)
    ));
    assert_eq!(selected.resources().counts(), before);
    let late_route = f.state.voice_channels.route(slot).unwrap();
    drop(output);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert_eq!(
        late_route.snapshot(),
        [1, 2, 1],
        "an already-held route can only finish inert closed Status"
    );
    assert!(f.state.voice_channels.route(slot).is_err());
    drop(late_route);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    let closed = selected.resources().counts();
    assert_eq!(closed.1, 0);
    let mut rng = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await.unwrap(),
        Status::Closed { .. }
    ));
    assert_eq!(rng.draws, 0);
    assert_eq!(selected.resources().counts(), closed);
    assert!(
        f.state
            .voice_channels
            .index
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows[usize::from(slot - 1)]
        .is_some(),
        "a live original lease keeps its irreversible close fence"
    );
    f.state.voice_authority.lock().unwrap().end(&owner);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(Option::is_none));
    f.close().await;
}

#[tokio::test]
async fn more_than_sixty_four_real_selected_generations_reuse_slots_without_reset_or_stale_cleanup()
{
    let f = Fixture::new().await;
    let mut previous: Option<String> = None;
    for serial in 1..=ROWS + 4 {
        let (owner, selected) = select(&f, point()).await;
        let output = ready(
            ensure(&f.state, &owner, [9; 32], &mut Rng::new())
                .await
                .unwrap(),
        );
        assert_eq!(output.public().slot, 1);
        assert_eq!(u128::from_be_bytes(output.public().session), serial as u128);
        let current = selected.resources().counts();
        if let Some(old) = &previous {
            collect_channel(&f.state, old, [9; 32]).await.unwrap();
            assert_eq!(
                selected.resources().counts(),
                current,
                "stale exact cleanup cannot close the replacement slot"
            );
            let retry = ready(
                ensure(&f.state, &owner, [9; 32], &mut Rng::new())
                    .await
                    .unwrap(),
            );
            assert!(output.is_same(&retry));
        }
        drop(output);
        collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
        assert_eq!(selected.resources().counts().1, 0);
        f.state.voice_authority.lock().unwrap().end(&owner);
        collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
        assert!(f
            .state
            .voice_channels
            .index
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .all(Option::is_none));
        previous = Some(owner);
    }
    f.close().await;
}

#[tokio::test]
async fn final_collection_uses_original_capacity_under_full_process_and_crypto_pressure() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let resources = selected.resources();
    let group = selected.binding().scope.group;
    let output = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let route = f.state.voice_channels.route(output.public().slot).unwrap();
    let header = route.session().header().unwrap();
    drop(route);
    drop(output);
    let mut lanes = Vec::new();
    for _ in 0..2 {
        lanes.push(resources.reserve_crypto_lane(group).unwrap());
    }
    for _ in 0..4 {
        lanes.push(resources.reserve_crypto_lane([97; 32]).unwrap());
    }
    let group_bytes = resources.counts().0 - INDEX_BYTES;
    let mut pressure = vec![resources
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - group_bytes,
            ResourceKind::Retained,
        )
        .unwrap()];
    for value in 97..=104 {
        let left = 2 * 1024 * 1024 - resources.counts().0;
        if left == 0 {
            break;
        }
        pressure.push(
            resources
                .charge(
                    [value; 32],
                    left.min(catcoms_sync::voice_v2::resources::GROUP_BYTES),
                    ResourceKind::Retained,
                )
                .unwrap(),
        );
    }
    assert_eq!(resources.counts(), (2 * 1024 * 1024, 8));
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert_eq!(
        resources.counts().1,
        7,
        "held header retains control slot after setup retirement"
    );
    drop(header);
    assert_eq!(
        resources.counts().1,
        6,
        "actual last Session owner releases its original lane"
    );
    f.state.voice_authority.lock().unwrap().end(&owner);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(Option::is_none));
    for lane in lanes {
        lane.close();
    }
    assert_eq!(resources.counts().1, 0);
    drop(pressure);
    f.close().await;
}

#[tokio::test]
async fn lock_unlock_and_replacement_do_not_revive_or_collect_the_wrong_original_session() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let output = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let late = f.state.voice_channels.route(output.public().slot).unwrap();
    crate::register_ui_lock_request(&f.state);
    f.state.voice_authority.lock().unwrap().revoke_all();
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert_eq!(late.snapshot(), [1, 2, 1]);
    let generation = f
        .state
        .ui_session_generation
        .load(std::sync::atomic::Ordering::Acquire);
    crate::finalize_unlock_session(&f.state, generation, false)
        .await
        .unwrap();
    let mut rng = Rng::new();
    assert!(matches!(
        ensure(&f.state, &owner, [9; 32], &mut rng).await,
        Err(Refused::Binding)
    ));
    assert_eq!(rng.draws, 0);
    // Unlock creates a fresh controller generation. The fixture's cached pre-lock controller
    // is deliberately obsolete, just as a remounted browser must not adopt that controller.
    let controller = crate::prepare_voice_controller_inner(&f.state)
        .await
        .unwrap();
    crate::activate_voice_owner_inner(&f.state, controller.clone())
        .await
        .unwrap();
    let candidate = crate::prepare_voice_owner_inner(
        &f.state,
        controller,
        7,
        crate::channel_id("general").to_string(),
    )
    .await
    .unwrap();
    let (replacement, replacement_selected) = select_candidate(&f, candidate, point()).await;
    let current = ready(
        ensure(&f.state, &replacement, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    assert_ne!(output.public().session, current.public().session);
    let before = selected.resources().counts();
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert_eq!(selected.resources().counts(), before);
    assert!(Arc::ptr_eq(
        &current,
        &ready(
            ensure(&f.state, &replacement, [9; 32], &mut Rng::new())
                .await
                .unwrap()
        )
    ));
    assert!(replacement_selected.lease().is_live());
    drop(late);
    drop(output);
    drop(current);
    collect_channel(&f.state, &replacement, [9; 32])
        .await
        .unwrap();
    f.state.voice_authority.lock().unwrap().end(&replacement);
    collect_channel(&f.state, &replacement, [9; 32])
        .await
        .unwrap();
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(Option::is_none));
    f.close().await;
}

#[tokio::test]
async fn late_last_owner_drop_is_collected_before_controller_admission_under_full_pressure() {
    let f = Fixture::new().await;
    let (owner, selected) = select(&f, point()).await;
    let resources = selected.resources();
    let output = ready(
        ensure(&f.state, &owner, [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let route = f.state.voice_channels.route(output.public().slot).unwrap();
    let header = route.session().header().unwrap();
    drop(route);
    drop(output);
    f.state.voice_authority.lock().unwrap().end(&owner);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    drop(header); // Actual native IO drains AFTER the successful final-close reply.
    let mut pressure = Vec::new();
    for value in 97..=105 {
        let left = 2 * 1024 * 1024 - resources.counts().0;
        if left == 0 {
            break;
        }
        pressure.push(
            resources
                .charge(
                    [value; 32],
                    left.min(catcoms_sync::voice_v2::resources::GROUP_BYTES),
                    ResourceKind::Retained,
                )
                .unwrap(),
        );
    }
    assert_eq!(resources.counts().0, 2 * 1024 * 1024);
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .any(Option::is_some));
    // No selected join or new call grant exists yet. This actual production entry point must
    // run housekeeping first; requiring successful selected-key setup to reach it deadlocks.
    let controller = crate::prepare_voice_controller_inner(&f.state)
        .await
        .unwrap();
    assert!(f
        .state
        .voice_channels
        .index
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .all(Option::is_none));
    assert_eq!(resources.counts().0, 2 * 1024 * 1024 - ROW_BYTES);
    crate::activate_voice_owner_inner(&f.state, controller)
        .await
        .unwrap();
    drop(pressure);
    f.close().await;
}
