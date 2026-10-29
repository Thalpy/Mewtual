//! Actual native activation/key/Root publication precedes every physical setup.
//! No phase flip, dummy grant, SDK-drained boolean or old selected-key conversion.
use super::*;
use crate::voice_call_activation::Handle;
use catcoms_rt::{Clock, ManualClock};

mod parent_claim;
mod slot;

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
    let admission = crate::voice_admitted_key::admit_physical_inner(&f.state, &candidate, [9; 32])
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
        assert!(output.is_same(&retry));
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
        let history = handle.physical_history([9; 32]).unwrap();
        let terminal = history
            .terminal()
            .expect("actual Row collection, not End alone");
        history
            .acknowledge_receipt(history.receipt_id(terminal))
            .unwrap();
        crate::voice_admitted_call::collect_retired(&f.state);
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
    let weak = route.session().downgrade();
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
        weak.upgrade().is_some(),
        "the admitted custodian keeps the real Session until its explicit weak witness returns"
    );
    assert_eq!(resources.counts(), retained);
    drop(weak);
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
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

#[tokio::test]
async fn admitted_physical_history_survives_key_pruning_and_full_pressure_without_reclassifying_admission(
) {
    use crate::voice_call_activation::physical::PhysicalTerminal;
    use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let history = handle.physical_history([9; 32]).unwrap();
    assert!(
        history.terminal().is_none(),
        "a live unstarted intent is not negative proof"
    );
    assert!(handle.physical_history([8; 32]).is_none());
    assert!(!handle.no_physical_intent(), "binding is irreversible");
    let refusal = crate::voice_admitted_key::admit_inner(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    assert_eq!(
        refusal.kind, "refused",
        "the key-only factory cannot adopt a physical generation"
    );
    assert_eq!(refusal.owner, handle.id());
    assert_eq!(refusal.executor, hex::encode([9; 32]));
    assert!(refusal.permit.is_empty() && refusal.join.is_empty());
    end(&f, &clock, &handle).await;
    assert!(
        f.state
            .voice_authority
            .lock()
            .unwrap()
            .admitted_activation(handle.id())
            .is_some(),
        "early parent ACK cannot forget an unseen physical terminal"
    );
    let terminal = history
        .terminal()
        .expect("closed original, genuinely unstarted latch");
    assert_eq!(terminal, PhysicalTerminal::NotStarted);
    let receipt = history.receipt_id(terminal);
    let resources = crate::voice_runtime(&f.state).resources();
    let group = handle.binding().scope.group;
    let pressure = resources
        .charge(
            group,
            GROUP_BYTES - resources.group_counts(group).unwrap().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = resources.counts();
    for _ in 0..8 {
        let recovered = handle.physical_history([9; 32]).unwrap();
        assert!(recovered.is_same(&history));
        assert_eq!(recovered.terminal(), Some(terminal));
        assert_eq!(recovered.receipt_id(terminal), receipt);
        assert_eq!(
            resources.counts(),
            before,
            "known history needs no new payment"
        );
    }
    let mut wrong = receipt;
    wrong[0] ^= 1;
    assert!(history.acknowledge_receipt(wrong).is_err());
    assert!(!history.acknowledged());
    let mut rng = Rng::new();
    assert!(ensure_admitted(&f.state, handle.id(), [9; 32], &mut rng)
        .await
        .is_err());
    assert_eq!(
        rng.draws, 0,
        "a late original cannot bind after no-start proof"
    );
    assert_eq!(history.terminal(), Some(terminal));
    drop(pressure);
    history.acknowledge_receipt(receipt).unwrap();
    crate::voice_admitted_call::collect_retired(&f.state);
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(handle.id())
        .is_none());
    assert_eq!(
        history.terminal(),
        Some(terminal),
        "held original reply keeps SAME history paid"
    );
    f.close().await;
}

#[tokio::test]
async fn admitted_physical_collection_receipt_waits_actual_last_output_and_recovers_lost_reply() {
    use crate::voice_call_activation::physical::PhysicalTerminal;
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let history = handle.physical_history([9; 32]).unwrap();
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let serial = output.public().session;
    end(&f, &clock, &handle).await;
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    assert!(
        history.terminal().is_none(),
        "a close reply is not actual Row/output return"
    );
    drop(output);
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    let terminal = PhysicalTerminal::Collected(serial);
    assert_eq!(history.terminal(), Some(terminal));
    assert_ne!(history.terminal(), Some(PhysicalTerminal::NotStarted));
    let receipt = history.receipt_id(terminal);
    // Discard the first observer exactly as a lost collect/reply projection.
    drop(handle.physical_history([9; 32]).unwrap());
    let original = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(handle.id())
        .unwrap();
    let recovered = original.physical_history([9; 32]).unwrap();
    assert!(recovered.is_same(&history));
    assert_eq!(recovered.receipt_id(recovered.terminal().unwrap()), receipt);
    assert!(
        !original.no_physical_intent(),
        "collected is never reset to no-intent"
    );
    assert!(
        !original.collectible(),
        "collection does not acknowledge its lost reply"
    );
    recovered.acknowledge_receipt(receipt).unwrap();
    crate::voice_admitted_call::collect_retired(&f.state);
    assert!(f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(handle.id())
        .is_none());
    f.close().await;
}

#[test]
fn admitted_physical_history_full_graph_is_guarded_before_payment() {
    let bytes = crate::voice_call_activation::physical::physical_history_layout_bytes();
    assert!(
        bytes <= crate::voice_call_activation::physical::PHYSICAL_HISTORY_BYTES,
        "complete physical history requires {bytes}"
    );
}

#[tokio::test]
async fn admitted_unentered_physical_rpc_after_end_cannot_bind_or_adopt_a_replacement() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let controller = controller(&f).await;
    let candidate = crate::voice_admitted_call::prepare_inner(
        &f.state,
        controller.clone(),
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
    // No physical admit has entered. The actual retained parent (not map
    // absence) owns an irreversible initial discriminator before any SDK grant.
    let old = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(&candidate)
        .unwrap();
    assert!(
        !old.no_physical_intent(),
        "an open parent is no negative proof"
    );
    end(&f, &clock, &old).await;
    assert!(old.no_physical_intent());
    let no_intent =
        crate::voice_handoff::commands::fixture_physical_observation(&f.state, old.id(), [9; 32])
            .await
            .unwrap();
    assert!(
        crate::voice_handoff::commands::fixture_physical_observation(&f.state, old.id(), [9; 32])
            .await
            .is_err()
    );
    let body = no_intent.fixture_body_with_limit(256).unwrap();
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["kind"], "no-physical-intent");
    assert_eq!(body["owner"], old.id());
    assert_eq!(
        body.as_object().unwrap().len(),
        2,
        "no executor/join/SDK certificate is invented"
    );
    let resources = crate::voice_runtime(&f.state).resources();
    let before = resources.counts();
    assert!(
        crate::voice_admitted_key::admit_physical_inner(&f.state, old.id(), [9; 32])
            .await
            .is_err()
    );
    assert!(old.physical_history([9; 32]).is_none());
    assert_eq!(
        resources.counts(),
        before,
        "the delayed unentered RPC allocated nothing"
    );
    let replacement = activate(&f, &controller).await;
    assert_ne!(old.id(), replacement.id());
    assert!(old.no_physical_intent());
    assert!(!replacement.no_physical_intent());
    assert!(
        crate::voice_admitted_key::admit_physical_inner(&f.state, old.id(), [9; 32])
            .await
            .is_err()
    );
    assert!(replacement.physical_history([9; 32]).is_some());
    // Only explicit parent receipt ACK can release this no-intent activation.
    crate::voice_admitted_call::acknowledge_inner(&f.state, old.id())
        .await
        .unwrap();
    end(&f, &clock, &replacement).await;
    let history = replacement.physical_history([9; 32]).unwrap();
    let terminal = history.terminal().unwrap();
    history
        .acknowledge_receipt(history.receipt_id(terminal))
        .unwrap();
    crate::voice_admitted_call::collect_retired(&f.state);
    f.close().await;
}

#[tokio::test]
async fn admitted_key_only_to_physical_refusal_preserves_original_generation_and_call() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let controller = controller(&f).await;
    let candidate = crate::voice_admitted_call::prepare_inner(
        &f.state,
        controller,
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
    let handle = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(&candidate)
        .unwrap();
    let original = crate::voice_admitted_key::admit_inner(&f.state, &candidate, [9; 32])
        .await
        .unwrap();
    let resources = crate::voice_runtime(&f.state).resources();
    let before = resources.counts();
    let refused = crate::voice_admitted_key::admit_physical_inner(&f.state, &candidate, [9; 32])
        .await
        .unwrap();
    assert_eq!(refused.kind, "refused");
    assert_eq!(refused.owner, candidate);
    assert_eq!(refused.executor, hex::encode([9; 32]));
    assert!(refused.permit.is_empty() && refused.join.is_empty());
    assert!(
        handle.active(),
        "a wrong-purpose observer must not End the original call"
    );
    assert!(handle.physical_history([9; 32]).is_none());
    assert!(
        !handle.no_physical_intent(),
        "KeyOnly is not the initial NoIntentYet discriminator"
    );
    let recovered = crate::voice_admitted_key::admit_inner(&f.state, &candidate, [9; 32])
        .await
        .unwrap();
    assert_eq!(recovered.permit, original.permit);
    assert_eq!(recovered.join, original.join);
    assert_eq!(recovered.kind, "admitted");
    assert_eq!(resources.counts(), before);
    crate::voice_admitted_key::settle_inner(&f.state, &candidate, [9; 32])
        .await
        .unwrap();
    end(&f, &clock, &handle).await;
    crate::voice_admitted_call::acknowledge_inner(&f.state, &candidate)
        .await
        .unwrap();
    f.close().await;
}

#[tokio::test]
async fn admitted_physical_metadata_projection_is_one_original_until_actual_reply_drop_error_or_unwind(
) {
    use crate::voice_handoff::commands::fixture_physical_observation as observe;
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let resources = crate::voice_runtime(&f.state).resources();
    let first = observe(&f.state, handle.id(), [9; 32]).await.unwrap();
    let before = resources.counts();
    assert!(observe(&f.state, handle.id(), [9; 32]).await.is_err());
    assert_eq!(
        resources.counts(),
        before,
        "Busy neither clones nor funds another serializer"
    );
    drop(first);
    let failed = observe(&f.state, handle.id(), [9; 32]).await.unwrap();
    assert!(failed.fixture_body_with_limit(0).is_err());
    let retry = observe(&f.state, handle.id(), [9; 32]).await.unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _original = retry;
            panic!("actual reply dropped during serializer unwind");
        }))
        .is_err()
    );
    assert!(
        observe(&f.state, handle.id(), [8; 32]).await.is_err(),
        "wrong tuple drops only its own temporary slot"
    );
    let exact = observe(&f.state, handle.id(), [9; 32]).await.unwrap();
    assert_eq!(resources.counts(), before);
    drop(exact);
    end(&f, &clock, &handle).await;
    let history = handle.physical_history([9; 32]).unwrap();
    let terminal = history.terminal().unwrap();
    history
        .acknowledge_receipt(history.receipt_id(terminal))
        .unwrap();
    crate::voice_admitted_call::collect_retired(&f.state);
    f.close().await;
}

#[tokio::test]
async fn admitted_session_drop_inflight_is_not_collection_and_cannot_be_reaped_by_a_second_collector(
) {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let history = handle.physical_history([9; 32]).unwrap();
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let selected = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_published_key(&handle, [9; 32])
        .unwrap();
    let route = f
        .state
        .voice_channels
        .admitted_route(output.public().slot)
        .unwrap();
    let serial = output.public().session;
    let held = route.session().clone();
    let (at_drop, release) = held.fixture_pause_final_drop();
    drop(route);
    end(&f, &clock, &handle).await;
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    let resources = selected.resources();
    let before = resources.counts();
    let registry = f.state.voice_channels.clone();
    let ledger = f.state.voice_handoff.clone();
    let (done, finished) = tokio::sync::oneshot::channel();
    drop(std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(held);
            registry.collect_retired(&ledger);
        }));
        let _ = done.send(result);
    }));
    bounded(at_drop).await.unwrap();
    drop(output);
    // The last raw strong count is now0 INSIDE the actual Session destructor.
    // The independent in-flight owner/marker, not weak absence, keeps this Row.
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    assert!(f
        .state
        .voice_channels
        .admitted_identity(handle.id(), [9; 32]));
    assert_eq!(
        resources.counts(),
        before,
        "no control/bootstrap/Row fee returns before actual destruction"
    );
    assert!(
        history.terminal().is_none(),
        "a paused destructor cannot mint ActuallyCollected"
    );
    drop(release); // RAII also unblocks the producer if either assertion unwinds
    bounded(finished).await.unwrap().unwrap();
    assert!(!f
        .state
        .voice_channels
        .admitted_identity(handle.id(), [9; 32]));
    assert_eq!(
        resources.counts().0,
        before.0
            - ROW_BYTES
            - crate::voice_handoff::channel::CONTROL_BYTES
            - crate::voice_handoff::channel::BOOTSTRAP_BYTES
            - crate::voice_handoff::channel::BROWSER_CONTROL_BYTES
    );
    let terminal = history.terminal().unwrap();
    assert_eq!(
        terminal,
        crate::voice_call_activation::physical::PhysicalTerminal::Collected(serial)
    );
    history
        .acknowledge_receipt(history.receipt_id(terminal))
        .unwrap();
    crate::voice_admitted_call::collect_retired(&f.state);
    drop(selected);
    f.close().await;
}

#[tokio::test]
async fn admitted_closed_live_call_releases_session_crypto_while_independent_output_remains_held() {
    use catcoms_sync::voice_v2::resources::ResourceKind;
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
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
    let held = route.session().clone();
    let (at_drop, release) = held.fixture_pause_final_drop();
    drop(route);
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    assert!(
        handle.active(),
        "independent Session retirement must not wait for whole-call End"
    );
    let resources = crate::voice_runtime(&f.state).resources();
    let count = resources.counts().1;
    assert!(count > 0 && count <= 8);
    let mut other = Vec::new();
    for i in 0..(8 - count) {
        other.push(
            resources
                .charge([200 + (i / 4) as u8; 32], 1, ResourceKind::Crypto)
                .unwrap(),
        );
    }
    assert!(resources
        .charge([211; 32], 1, ResourceKind::Crypto)
        .is_err());
    let before = resources.counts();
    let registry = f.state.voice_channels.clone();
    let ledger = f.state.voice_handoff.clone();
    let (done, finished) = tokio::sync::oneshot::channel();
    drop(std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(held);
            registry.collect_retired(&ledger);
        }));
        let _ = done.send(result);
    }));
    bounded(at_drop).await.unwrap();
    assert_eq!(resources.counts(), before);
    drop(release);
    bounded(finished).await.unwrap().unwrap();
    assert!(
        f.state
            .voice_channels
            .admitted_identity(handle.id(), [9; 32]),
        "live lease and actual held output still retain Row"
    );
    assert_eq!(
        resources.counts().0,
        before.0
            - crate::voice_handoff::channel::CONTROL_BYTES
            - crate::voice_handoff::channel::BOOTSTRAP_BYTES
            - crate::voice_handoff::channel::BROWSER_CONTROL_BYTES
    );
    let progress = resources
        .charge([211; 32], 1, ResourceKind::Crypto)
        .unwrap();
    drop((progress, other));
    end(&f, &clock, &handle).await;
    drop(output);
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff);
    f.close().await;
}
