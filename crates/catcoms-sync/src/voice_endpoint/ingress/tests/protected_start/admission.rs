//! Genuine Root participant/Actor/pool admission; no native IPC or agreement is implied.
use super::*;
use voice_v2::admission::REGISTRATION_METADATA_BYTES;

#[tokio::test]
async fn admission_step_preserves_the_minted_registration_when_high_capacity_is_full() {
    use voice_v2::admission::AdmissionRpcPermit;
    let (mut root, _remote, runtime, clock, grant) = low_discovery_fixture(1).await;
    clock.advance_ms(60_000);
    runtime.collect_expired();
    let resources = runtime.resources();
    let payment = AdmissionRpcPermit::reserve(&resources, grant.scope()).unwrap();
    let before = resources.counts();
    let pressure = resources
        .charge(
            grant.scope().group,
            voice_v2::resources::GROUP_BYTES - before.0 - REGISTRATION_METADATA_BYTES,
            voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let prepared = root
        .prepare_voice_admission_step(&grant, None, 1, &payment)
        .unwrap();
    let (registration, capacity) = prepared.into_parts();
    assert_eq!(capacity.err(), Some(VoiceError::Capacity));
    let registration = registration.expect("the original mint survives HIGH refusal");
    let incarnation = registration.incarnation();
    drop(pressure);
    let observed = registration.observer();
    let retry = root
        .prepare_voice_admission_step(&grant, Some(&observed), 1, &payment)
        .unwrap();
    let (replacement, capacity) = retry.into_parts();
    assert!(
        replacement.is_none(),
        "capacity retry cannot mint another registration"
    );
    let capacity = capacity.unwrap();
    assert!(capacity.belongs_to(&registration));
    assert_eq!(registration.incarnation(), incarnation);
    drop(capacity);
    drop(observed);
    drop(registration);
    assert_eq!(resources.counts(), before);
}

#[tokio::test]
async fn admission_step_refuses_foreign_rpc_pool_scope_and_zero_before_any_mint() {
    use voice_v2::admission::AdmissionRpcPermit;
    let (mut root, _remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let resources = runtime.resources();
    let before = resources.counts();
    let foreign = Arc::new(voice_v2::resources::Resources::default());
    let payment = AdmissionRpcPermit::reserve(&foreign, grant.scope()).unwrap();
    assert_eq!(
        root.prepare_voice_admission_step(&grant, None, 1, &payment)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(resources.counts(), before);
    assert_eq!(
        root.prepare_voice_admission_step(&grant, None, 0, &payment)
            .err(),
        Some(VoiceError::Bounds)
    );
    assert_eq!(resources.counts(), before);
    drop(payment);
    assert_eq!(foreign.counts(), (0, 0));
    let mut scope = grant.scope();
    scope.room += 1;
    let payment = AdmissionRpcPermit::reserve(&resources, scope).unwrap();
    let held = resources.counts();
    assert_eq!(
        root.prepare_voice_admission_step(&grant, None, 1, &payment)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(resources.counts(), held);
    drop(payment);
    assert_eq!(resources.counts(), before);
}

#[tokio::test]
async fn admission_registration_identity_is_stable_and_observers_keep_actual_metadata_paid() {
    let (mut root, _remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let pool = runtime.resources();
    let before = pool.counts();
    let owner = root.prepare_voice_admission_session(&grant).unwrap();
    assert_eq!(owner.local_device(), root.device_id());
    let original = owner.incarnation();
    let first = owner.observer();
    let last = first.clone();
    assert_eq!(first.incarnation(), original);
    assert!(owner.owns(&first) && owner.owns(&last));
    let held = (before.0 + REGISTRATION_METADATA_BYTES, before.1);
    assert_eq!(pool.counts(), held);
    first.cancel_through(1).unwrap();
    assert_eq!(
        owner.incarnation(),
        original,
        "cancel does not mint a new session"
    );
    drop(owner);
    assert!(!last.is_current());
    assert!(
        grant.lease().is_live(),
        "registration close does not end the call"
    );
    assert_eq!(pool.counts(), held);
    drop(first);
    assert_eq!(pool.counts(), held);
    drop(last);
    assert_eq!(pool.counts(), before);
}

#[tokio::test]
async fn admission_metadata_survives_the_actual_last_registration_body_return() {
    let (mut root, _remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let pool = runtime.resources();
    let before = pool.counts();
    let owner = root.prepare_voice_admission_session(&grant).unwrap();
    let last = owner.observer();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    // These barriers belong to the test harness, not an application owner.
    owner.fixture_pause_final_body_return(entered.clone(), resume.clone());
    drop(owner);
    let retiring = std::thread::spawn(move || drop(last));
    entered.wait();
    assert_eq!(
        pool.counts(),
        (before.0 + REGISTRATION_METADATA_BYTES, before.1)
    );
    resume.wait();
    retiring.join().unwrap();
    assert_eq!(pool.counts(), before);
}

#[tokio::test]
async fn cancelled_high_preclaim_stays_paid_until_drop_and_old_cancel_cannot_close_its_successor() {
    let (mut root, _remote, runtime, clock, grant) = low_discovery_fixture(1).await;
    // Retire the fixture's earlier discovery traffic BEFORE measuring this inert
    // reservation. Otherwise its unrelated expiry could change the same pool tally.
    clock.advance_ms(60_000);
    runtime.collect_expired();
    let owner = root.prepare_voice_admission_session(&grant).unwrap();
    let observed = owner.observer();
    let id = owner.incarnation();
    let pool = runtime.resources();
    let before = pool.counts();
    observed.cancel_through(1).unwrap();
    assert_eq!(
        root.reserve_voice_high_preclaim(&grant, &observed, 1).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(pool.counts(), before);
    let first = root
        .reserve_voice_high_preclaim(&grant, &observed, 2)
        .unwrap();
    assert!(first.belongs_to(&owner));
    assert_eq!(first.generation(), 2);
    let waiting = pool.counts();
    clock.advance_ms(60_000);
    runtime.collect_expired();
    assert!(
        first.is_current(),
        "inert waiting starts no15s transcript deadline"
    );
    assert_eq!(pool.counts(), waiting);
    let permit =
        voice_v2::crypto_actions::CryptoActionSnapshotPermit::new(&pool, grant.scope()).unwrap();
    let actions = root.voice_confirmation_actions(&grant, permit);
    assert!(
        actions.result().unwrap().is_empty(),
        "inert HIGH capacity creates no crypto Action"
    );
    drop(actions);
    assert_eq!(pool.counts(), waiting);
    let held = pool.counts();
    observed.cancel_through(2).unwrap();
    assert!(!first.is_current());
    assert_eq!(
        pool.counts(),
        held,
        "cancellation is not actual capacity return"
    );
    assert_eq!(
        root.reserve_voice_high_preclaim(&grant, &observed, 3).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), held);
    drop(first);
    assert_eq!(pool.counts(), before);
    let next = root
        .reserve_voice_high_preclaim(&grant, &observed, 3)
        .unwrap();
    observed.cancel_through(2).unwrap();
    assert!(next.is_current(), "old cancellation cannot clear g+1");
    assert_eq!(owner.incarnation(), id);
    observed.cancel_through(u64::MAX).unwrap();
    assert!(!next.is_current());
    assert!(!observed.is_current());
    assert!(grant.lease().is_live());
    drop(next);
    assert_eq!(pool.counts(), before);
}

#[tokio::test]
async fn high_preclaim_pressure_is_unstarted_and_wrong_root_or_stopped_join_cannot_supply_its_pools(
) {
    let (mut root, _remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let owner = root.prepare_voice_admission_session(&grant).unwrap();
    let original = owner.observer();
    let pool = runtime.resources();
    let before = pool.counts();
    let pressure = pool
        .charge(
            grant.scope().group,
            voice_v2::resources::GROUP_BYTES - before.0,
            voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    assert_eq!(
        root.reserve_voice_high_preclaim(&grant, &original, 1).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), full);
    drop(pressure);
    assert_eq!(pool.counts(), before);
    let high = root
        .reserve_voice_high_preclaim(&grant, &original, 1)
        .unwrap();
    drop(high);
    assert_eq!(pool.counts(), before);
    let (other, _other_remote, other_runtime, _other_clock, other_grant) =
        low_discovery_fixture(1).await;
    let other_before = other_runtime.resources().counts();
    assert_eq!(
        other
            .reserve_voice_high_preclaim(&other_grant, &original, 1)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(other_runtime.resources().counts(), other_before);
    root.stop_voice_endpoint(&grant);
    let stopped = pool.counts();
    assert!(!original.is_current());
    assert_eq!(
        root.reserve_voice_high_preclaim(&grant, &original, 2).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(pool.counts(), stopped);
}

#[tokio::test]
async fn distinct_valid_registrations_with_equal_labels_do_not_share_identity_or_cancel_floors() {
    let (mut root, _remote, _runtime, _clock, grant) = low_discovery_fixture(1).await;
    let first = root.prepare_voice_admission_session(&grant).unwrap();
    let other = root.prepare_voice_admission_session(&grant).unwrap();
    let observed = first.observer();
    let foreign = other.observer();
    assert!(!first.owns(&foreign));
    assert_ne!(first.incarnation(), other.incarnation());
    let high = root
        .reserve_voice_high_preclaim(&grant, &observed, 1)
        .unwrap();
    foreign.cancel_through(1).unwrap();
    assert!(high.is_current());
    assert!(!high.belongs_to(&other));
    assert_eq!(observed.cancel_through(0), Err(VoiceError::Bounds));
    drop(other);
    assert!(
        high.is_current(),
        "another registration cannot close this original"
    );
}

#[tokio::test]
async fn admission_final_allocation_fence_rejects_actual_grant_and_session_revocation() {
    for revoke_session in [false, true] {
        let (mut root, _remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
        let pool = runtime.resources();
        let before = pool.counts();
        let locks_free = root
            .voice_runtime
            .as_ref()
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .lock_probe_for_test();
        let reached = std::cell::Cell::new(false);
        let result = root.prepare_voice_admission_session_at(&grant, || {
            reached.set(true);
            assert!(
                locks_free(),
                "allocation callback must precede Group/Policy guards"
            );
            assert_eq!(
                pool.counts(),
                (before.0 + REGISTRATION_METADATA_BYTES, before.1)
            );
            if revoke_session {
                runtime.revoke_session();
            } else {
                grant.lease().revoke();
            }
        });
        assert!(
            reached.get(),
            "must cross the real post-allocation boundary"
        );
        assert_eq!(result.err(), Some(VoiceError::Unauthorized));
        assert_eq!(
            pool.counts(),
            before,
            "unpublished owner rolls back exactly"
        );
    }
}

#[tokio::test]
async fn admission_allocation_boundary_unwind_keeps_no_unpublished_owner_or_payment() {
    let (mut root, _remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let pool = runtime.resources();
    let before = pool.counts();
    let reached = std::cell::Cell::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        root.prepare_voice_admission_session_at(&grant, || {
            reached.set(true);
            assert_eq!(
                pool.counts(),
                (before.0 + REGISTRATION_METADATA_BYTES, before.1)
            );
            panic!("controlled allocation boundary unwind");
        })
    }));
    assert!(reached.get());
    assert!(result.is_err());
    assert_eq!(pool.counts(), before);
    assert!(
        grant.lease().is_live(),
        "unpublished registration owns no call close"
    );
}

#[tokio::test]
async fn admission_room_delete_recreate_cannot_revive_held_original_capacity() {
    use automerge::{transaction::Transactable, ROOT};
    let (mut root, _remote, runtime, clock, grant) = low_discovery_fixture(2).await;
    clock.advance_ms(60_000);
    runtime.collect_expired();
    let owner = root.prepare_voice_admission_session(&grant).unwrap();
    let original = owner.observer();
    let pool = runtime.resources();
    let before = pool.counts();
    let high = root
        .reserve_voice_high_preclaim(&grant, &original, 1)
        .unwrap();
    let held = pool.counts();
    root.post(DocType::ChannelIndex, 991, |doc| doc.delete(ROOT, "2"))
        .await
        .unwrap();
    assert!(!grant.lease().is_live());
    assert!(!owner.is_current() && !original.is_current() && !high.is_current());
    assert_eq!(
        pool.counts(),
        held,
        "room revocation is not capacity return"
    );
    root.post(DocType::ChannelIndex, 991, |doc| doc.put(ROOT, "2", true))
        .await
        .unwrap();
    let replacement = root.voice_call_grant(2).unwrap();
    assert!(replacement.lease().is_live());
    assert!(
        !high.is_current(),
        "equal room labels cannot revive the old allocation"
    );
    let with_replacement = pool.counts();
    drop(high);
    assert_eq!(
        pool.counts(),
        (
            with_replacement.0 - (held.0 - before.0),
            with_replacement.1 - (held.1 - before.1)
        )
    );
}

#[tokio::test]
async fn admission_runtime_lock_unlock_never_revives_old_registration_or_refunds_held_capacity() {
    let (mut root, _remote, runtime, clock, grant) = low_discovery_fixture(1).await;
    clock.advance_ms(60_000);
    runtime.collect_expired();
    let owner = root.prepare_voice_admission_session(&grant).unwrap();
    let original = owner.observer();
    let pool = runtime.resources();
    let before = pool.counts();
    let high = root
        .reserve_voice_high_preclaim(&grant, &original, 1)
        .unwrap();
    let held = pool.counts();
    runtime.revoke_session();
    assert!(!owner.is_current() && !original.is_current() && !high.is_current());
    assert_eq!(pool.counts(), held);
    attach(&mut root, &runtime);
    let replacement = Arc::new(root.voice_call_grant(1).unwrap());
    assert!(replacement.lease().is_live());
    assert!(!high.is_current());
    assert_eq!(
        root.reserve_voice_high_preclaim(&replacement, &original, 2)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let with_replacement = pool.counts();
    drop(high);
    assert_eq!(
        pool.counts(),
        (
            with_replacement.0 - (held.0 - before.0),
            with_replacement.1 - (held.1 - before.1)
        )
    );
}
