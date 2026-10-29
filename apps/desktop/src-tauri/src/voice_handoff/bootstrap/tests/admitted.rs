//! Actual native activation/key/Root publication precedes every physical setup.
//! No phase flip, dummy grant, SDK-drained boolean or old selected-key conversion.
use super::*;
use crate::voice_call_activation::Handle;
use catcoms_rt::{Clock, ManualClock};

mod parent_claim;

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::select! {
        value = future => value,
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => panic!("admitted bootstrap exceeded its bound"),
    }
}
async fn controller(f: &Fixture) -> String {
    let candidate = f.candidate().await;
    let authority = f.state.voice_authority.lock().unwrap();
    let crate::voice_authority::CandidateKind::Call { controller, .. } = authority
        .candidate(&candidate, catcoms_rt::SystemClock.monotonic_ms())
        .unwrap()
    else {
        panic!("fixture controller")
    };
    controller
}
async fn activate(f: &Fixture, controller: &str) -> Handle {
    let candidate = crate::voice_admitted_call::prepare_inner(
        &f.state,
        controller.to_owned(),
        7,
        catcoms_app::channel_id("general").to_string(),
    )
    .await
    .unwrap();
    bounded(crate::voice_admitted_call::activate_inner(
        &f.state,
        candidate.clone(),
    ))
    .await
    .unwrap();
    crate::voice_admitted_call::acknowledge_inner(&f.state, &candidate)
        .await
        .unwrap();
    let admission = crate::voice_admitted_key::admit_inner(&f.state, &candidate, [9; 32])
        .await
        .unwrap();
    bounded(crate::voice_admitted_key::publish_inner(
        &f.state,
        &candidate,
        [9; 32],
        &admission.permit,
        point(),
    ))
    .await
    .unwrap();
    let pending = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_publication_task_for_cleanup(&candidate, [9; 32])
        .unwrap();
    // A terminal JSON/native result is not execution Box destruction. Pin the
    // actual original return marker before making exact physical fee assertions.
    bounded(async {
        while !pending.fixture_task_returned() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    crate::voice_admitted_key::acknowledge_inner(&f.state, &candidate, [9; 32]).unwrap();
    crate::voice_admitted_key::settle_inner(&f.state, &candidate, [9; 32])
        .await
        .unwrap();
    f.state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(&candidate)
        .unwrap()
}
async fn end(f: &Fixture, clock: &ManualClock, handle: &Handle) {
    crate::end_voice_owner_inner(&f.state, handle.id()).await;
    clock.advance_ms(250);
    bounded(handle.wait_execution_return()).await;
    crate::voice_admitted_call::collect_retired(&f.state);
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
}

#[tokio::test]
async fn admitted_physical_setup_recovers_one_original_output_and_refuses_every_existing_route() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let mut rng = Rng::new();
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut rng)
            .await
            .unwrap(),
    );
    assert!(rng.draws > 0);
    let before = crate::voice_runtime(&f.state).resources().counts();
    for _ in 0..64 {
        let mut unused = Rng::new();
        let retry = ready(
            ensure_admitted(&f.state, handle.id(), [9; 32], &mut unused)
                .await
                .unwrap(),
        );
        assert!(Arc::ptr_eq(&output, &retry));
        assert_eq!(unused.draws, 0);
    }
    assert_eq!(crate::voice_runtime(&f.state).resources().counts(), before);
    assert!(output
        .public()
        .transcript()
        .starts_with(b"Mewtual admitted native voice bootstrap\0\x02\x01\x01"));
    assert!(
        f.state.voice_channels.route(output.public().slot).is_err(),
        "old1..30 dispatch receives no Existing Route"
    );
    assert!(f
        .state
        .voice_channels
        .admitted_route(output.public().slot)
        .is_ok());
    assert!(f
        .state
        .voice_channels
        .project(handle.id(), [9; 32])
        .is_err());
    let projection = f
        .state
        .voice_channels
        .project_admitted(handle.id(), [9; 32])
        .unwrap();
    assert!(projection.existing_selected().is_err());
    drop(projection);
    assert!(matches!(
        ensure(&f.state, handle.id(), [9; 32], &mut Rng::new()).await,
        Err(Refused::Binding)
    ));
    assert!(matches!(
        settle_setup(&f.state, handle.id(), [9; 32]).await,
        Err(Refused::Binding)
    ));
    assert!(matches!(
        collect_channel(&f.state, handle.id(), [9; 32]).await,
        Err(Refused::Binding)
    ));
    assert_eq!(crate::voice_runtime(&f.state).resources().counts(), before);
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    drop(output);
    end(&f, &clock, &handle).await;
    f.close().await;
}

#[tokio::test]
async fn admitted_close_after_end_joins_original_output_and_missing_history_is_not_a_certificate() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let resources = crate::voice_runtime(&f.state).resources();
    let before_selection = resources.counts().0;
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_published_key(&handle, [9; 32])
        .unwrap();
    let selected_fee = resources.counts().0 - before_selection;
    assert!(selected_fee > 0);
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let before = resources.counts();
    assert!(matches!(
        settle_admitted_setup(&f.state, handle.id(), [8; 32]).await,
        Err(Refused::Binding)
    ));
    assert_eq!(resources.counts(), before);
    end(&f, &clock, &handle).await;
    crate::register_ui_lock_request(&f.state);
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    assert!(
        f.state
            .voice_channels
            .admitted_identity(handle.id(), [9; 32]),
        "actual public output retains its paid Row"
    );
    let retained = resources.counts();
    drop(output);
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    assert!(!f
        .state
        .voice_channels
        .admitted_identity(handle.id(), [9; 32]));
    assert_eq!(resources.counts().0, retained.0 - ROW_BYTES);
    let before_selection_drop = resources.counts().0;
    drop(selected);
    assert_eq!(resources.counts().0, before_selection_drop - selected_fee);
    let before = resources.counts();
    assert!(matches!(
        collect_admitted_channel(&f.state, handle.id(), [9; 32]).await,
        Err(Refused::Binding)
    ));
    assert_eq!(
        resources.counts(),
        before,
        "unknown/collected history creates no row or no-work proof"
    );
    f.close().await;
}

#[tokio::test]
async fn admitted_registered_lock_or_end_during_real_curve_work_cannot_publish_or_redraw() {
    for lock in [false, true] {
        let clock = Arc::new(ManualClock::new(1000));
        let f = Fixture::with_voice_clock(clock.clone()).await;
        let handle = activate(&f, &controller(&f).await).await;
        let mut rng = Rng::new();
        rng.hook = Some(Box::new(|| {
            if lock {
                crate::register_ui_lock_request(&f.state);
            } else {
                handle.close();
            }
        }));
        assert!(matches!(
            ensure_admitted(&f.state, handle.id(), [9; 32], &mut rng).await,
            Err(Refused::Binding)
        ));
        assert!(rng.draws > 0);
        drop(rng);
        let before = crate::voice_runtime(&f.state).resources().counts();
        let mut unused = Rng::new();
        assert!(ensure_admitted(&f.state, handle.id(), [9; 32], &mut unused)
            .await
            .is_err());
        assert_eq!(unused.draws, 0);
        assert_eq!(crate::voice_runtime(&f.state).resources().counts(), before);
        settle_admitted_setup(&f.state, handle.id(), [9; 32])
            .await
            .unwrap();
        collect_admitted_channel(&f.state, handle.id(), [9; 32])
            .await
            .unwrap();
        end(&f, &clock, &handle).await;
        f.close().await;
    }
}

#[tokio::test]
async fn admitted_real_full_group_refusal_preserves_the_original_key_and_serial() {
    use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_published_key(&handle, [9; 32])
        .unwrap();
    let resources = selected.resources();
    let group = selected.binding().scope.group;
    let charged = resources.group_counts(group).unwrap().0;
    let pressure = resources
        .charge(group, GROUP_BYTES - charged, ResourceKind::Retained)
        .unwrap();
    assert_eq!(resources.group_counts(group).unwrap().0, GROUP_BYTES);
    let mut rng = Rng::new();
    assert!(matches!(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut rng).await,
        Err(Refused::Capacity)
    ));
    assert_eq!(rng.draws, 0);
    assert_eq!(
        f.state
            .voice_channels
            .index
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .serial,
        0
    );
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_published_key(&handle, [9; 32])
        .unwrap()
        .is_same(&selected));
    drop(pressure);
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut rng)
            .await
            .unwrap(),
    );
    assert_eq!(output.public().session, 1u128.to_be_bytes());
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    drop(output);
    end(&f, &clock, &handle).await;
    f.close().await;
}

#[tokio::test]
async fn admitted_sixty_five_real_calls_reuse_the_same_bounded_registry_without_counter_reset() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let controller = controller(&f).await;
    for serial in 1u128..=65 {
        let handle = activate(&f, &controller).await;
        let output = ready(
            ensure_admitted(&f.state, handle.id(), [9; 32], &mut Rng::new())
                .await
                .unwrap(),
        );
        assert_eq!(output.public().session, serial.to_be_bytes());
        assert_eq!(
            output.public().slot,
            1,
            "actual returned slot may be reused, serial cannot"
        );
        settle_admitted_setup(&f.state, handle.id(), [9; 32])
            .await
            .unwrap();
        collect_admitted_channel(&f.state, handle.id(), [9; 32])
            .await
            .unwrap();
        drop(output);
        end(&f, &clock, &handle).await;
        let held = f.state.voice_channels.index.lock().unwrap();
        let index = held.as_ref().unwrap();
        assert_eq!(index.rows.len(), 64);
        assert!(index.rows.iter().all(Option::is_none));
        assert_eq!(index.serial, serial);
    }
    f.close().await;
}

#[tokio::test]
async fn admitted_actual_weak_session_tail_keeps_the_original_row_payer_until_output_return() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let resources = crate::voice_runtime(&f.state).resources();
    let before_selection = resources.counts().0;
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_published_key(&handle, [9; 32])
        .unwrap();
    let selected_fee = resources.counts().0 - before_selection;
    assert!(selected_fee > 0);
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let route = f
        .state
        .voice_channels
        .admitted_route(output.public().slot)
        .unwrap();
    let weak = Arc::downgrade(route.session());
    end(&f, &clock, &handle).await;
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    let retained = resources.counts();
    assert!(
        weak.upgrade().is_some(),
        "the actual route still owns its Session"
    );
    drop(route);
    assert!(
        weak.upgrade().is_none(),
        "actual last strong Session returned"
    );
    assert_eq!(
        resources.counts().0,
        retained.0
            - crate::voice_handoff::channel::CONTROL_BYTES
            - crate::voice_handoff::channel::BOOTSTRAP_BYTES
            - crate::voice_handoff::channel::BROWSER_CONTROL_BYTES
    );
    let retained = resources.counts();
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    assert!(
        f.state
            .voice_channels
            .admitted_identity(handle.id(), [9; 32]),
        "weak-only tail and held output still have the Row payer"
    );
    drop(weak); // caller's own weak witness also returns before collection
    drop(output);
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    assert!(!f
        .state
        .voice_channels
        .admitted_identity(handle.id(), [9; 32]));
    assert_eq!(resources.counts().0, retained.0 - ROW_BYTES);
    let before_selection_drop = resources.counts().0;
    drop(selected);
    assert_eq!(resources.counts().0, before_selection_drop - selected_fee);
    f.close().await;
}
