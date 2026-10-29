//! Actual admitted Root/issuer fixture; no renderer roles or invented rounds.
use super::*;
use crate::voice_v2::admission::AdmissionRpcPermit;
use crate::voice_v2::admitted_admission::test_support::{bounded, Producer};
use crate::voice_v2::admitted_admission::{POOLS_BYTES, REGISTRATION_BYTES};
use crate::voice_v2::presence::WATCHER_BYTES;
use crate::voice_v2::resources::{ResourceKind, GROUP_BYTES};

#[tokio::test]
async fn admitted_neutral_requires_native_active_and_actual_published_presence_before_allocating() {
    let (mut node, runtime, _clock, old_grant, _old_owner, old, key, _remote) = fixture().await;
    node.clear_admitted_voice_call(&old).unwrap();
    let (grant, owner) = node.prepare_admitted_voice_call(1).unwrap().into_parts();
    let activation = owner.observer();
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    for selected in [false, true] {
        let selection = if selected {
            Some(
                node.select_admitted_voice_call(&grant, &activation)
                    .unwrap(),
            )
        } else {
            None
        };
        let before = runtime.resources().counts();
        let mut expected_rng = node.rng.clone();
        assert_eq!(
            node.prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
                .err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(runtime.resources().counts(), before);
        assert_eq!(
            rand_core::RngCore::next_u64(&mut node.rng),
            rand_core::RngCore::next_u64(&mut expected_rng)
        );
        if let Some(selection) = selection {
            selection.commit_native(&grant).unwrap();
        }
    }
    let before = runtime.resources().counts();
    assert!(activation.is_active());
    assert_eq!(
        node.prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        runtime.resources().counts(),
        before,
        "NativeActive alone cannot create Presence or pools"
    );
    node.start_admitted_voice_presence(grant.clone(), activation.clone(), key.join, key.public_key)
        .unwrap();
    let (registration, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    assert!(registration.unwrap().is_current());
    assert!(capacity.is_ok());
    drop(old_grant);
}

#[tokio::test]
async fn admitted_neutral_root_prepares_original_registration_and_actual_six_six_pools() {
    let (mut node, runtime, _clock, grant, _owner, activation, _key, _remote) = fixture().await;
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let (session, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    let session = session.unwrap();
    let capacity = capacity.unwrap();
    let observed = session.observer();
    assert_eq!(
        session.local_device(),
        node.voice_v2_context(1).unwrap().local
    );
    assert!(session.is_current() && capacity.is_current() && capacity.belongs_to(&session));
    assert_eq!(capacity.generation(), 1);
    assert_eq!(observed.unreserved_counts(), (20, 26, 26));
    drop(capacity);
    assert_eq!(observed.unreserved_counts(), (32, 32, 32));
    let mut expected_rng = node.rng.clone();
    let (fresh, retry) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, Some(&observed), 1, &payment)
        .unwrap()
        .into_parts();
    assert!(fresh.is_none(), "retry must not mint a second registration");
    assert!(retry.unwrap().belongs_to(&session));
    assert_eq!(
        rand_core::RngCore::next_u64(&mut node.rng),
        rand_core::RngCore::next_u64(&mut expected_rng)
    );
}

#[tokio::test]
async fn admitted_neutral_capacity_pressure_preserves_same_registration_without_rng_or_new_pools() {
    let (mut node, runtime, _clock, grant, _owner, activation, _key, _remote) = fixture().await;
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let used = runtime
        .resources()
        .group_counts(grant.scope().group)
        .unwrap()
        .0;
    let pressure = runtime
        .resources()
        .charge(
            grant.scope().group,
            GROUP_BYTES - used - POOLS_BYTES - REGISTRATION_BYTES,
            ResourceKind::Retained,
        )
        .unwrap();
    let (session, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    assert_eq!(capacity.err(), Some(VoiceError::Capacity));
    let session = session.expect("registration survives later capacity refusal");
    let original = session.observer();
    let identity = original.incarnation();
    let counts = runtime.resources().counts();
    let mut expected_rng = node.rng.clone();
    for _ in 0..8 {
        let (fresh, capacity) = node
            .prepare_admitted_voice_admission_step(
                &grant,
                &activation,
                Some(&original),
                1,
                &payment,
            )
            .unwrap()
            .into_parts();
        assert!(fresh.is_none());
        assert_eq!(capacity.err(), Some(VoiceError::Capacity));
        assert_eq!(original.incarnation(), identity);
        assert_eq!(runtime.resources().counts(), counts);
    }
    assert_eq!(
        rand_core::RngCore::next_u64(&mut node.rng),
        rand_core::RngCore::next_u64(&mut expected_rng)
    );
    drop(pressure);
    let (fresh, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, Some(&original), 1, &payment)
        .unwrap()
        .into_parts();
    assert!(fresh.is_none());
    assert!(capacity.unwrap().belongs_to(&session));
    assert_eq!(original.incarnation(), identity);
}

#[tokio::test]
async fn admitted_neutral_foreign_root_observer_and_payment_cannot_allocate_or_adopt_pools() {
    let (mut a, runtime_a, _clock_a, grant_a, _owner_a, active_a, _key_a, _remote_a) =
        fixture().await;
    let payment_a = AdmissionRpcPermit::reserve(&runtime_a.resources(), grant_a.scope()).unwrap();
    let (session, capacity) = a
        .prepare_admitted_voice_admission_step(&grant_a, &active_a, None, 1, &payment_a)
        .unwrap()
        .into_parts();
    drop(capacity.unwrap());
    let session = session.unwrap();
    let (mut b, runtime_b, _clock_b, grant_b, _owner_b, active_b, _key_b, _remote_b) =
        fixture().await;
    let payment_b = AdmissionRpcPermit::reserve(&runtime_b.resources(), grant_b.scope()).unwrap();
    let before = runtime_b.resources().counts();
    assert_eq!(
        b.prepare_admitted_voice_admission_step(&grant_b, &active_b, None, 1, &payment_a)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let (fresh, capacity) = b
        .prepare_admitted_voice_admission_step(
            &grant_b,
            &active_b,
            Some(&session.observer()),
            1,
            &payment_b,
        )
        .unwrap()
        .into_parts();
    assert!(fresh.is_none());
    assert_eq!(capacity.err(), Some(VoiceError::Unauthorized));
    assert_eq!(runtime_b.resources().counts(), before);
}

#[tokio::test]
async fn admitted_neutral_cancellation_is_monotone_and_scoped_without_closing_call() {
    let (mut node, runtime, _clock, grant, _owner, activation, _key, _remote) = fixture().await;
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let (session, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    let session = session.unwrap();
    let original = session.observer();
    let capacity = capacity.unwrap();
    assert_eq!(original.cancel_through(0), Err(VoiceError::Bounds));
    assert!(capacity.is_current());
    original.cancel_through(1).unwrap();
    assert!(!capacity.is_current());
    assert!(activation.is_active(), "generation cancellation is not End");
    drop(capacity);
    let (_, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, Some(&original), 1, &payment)
        .unwrap()
        .into_parts();
    assert_eq!(capacity.err(), Some(VoiceError::Unauthorized));
    let (_, successor) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, Some(&original), 2, &payment)
        .unwrap()
        .into_parts();
    assert_eq!(successor.unwrap().generation(), 2);
    original.cancel_through(u64::MAX).unwrap();
    assert!(!session.is_current());
    assert!(activation.is_active());
}

#[tokio::test]
async fn admitted_neutral_phase_first_negative_refuses_even_before_live_bit_and_keeps_custody() {
    let (mut node, runtime, _clock, grant, _owner, activation, _key, _remote) = fixture().await;
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let (session, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    let session = session.unwrap();
    let capacity = capacity.unwrap();
    let charged = runtime.resources().counts();
    let lease = grant.lease();
    lease
        .activation_phase()
        .store(crate::voice_v2::call_activation::CLOSED, Ordering::Release);
    assert!(lease.is_live());
    assert!(!session.is_current() && !capacity.is_current());
    assert_eq!(
        runtime.resources().counts(),
        charged,
        "phase closure does not return actual capacity"
    );
    assert_eq!(
        node.prepare_admitted_voice_admission_step(
            &grant,
            &activation,
            Some(&session.observer()),
            2,
            &payment
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
}

#[tokio::test]
async fn admitted_neutral_real_mls_epoch_change_retires_old_registration_and_preclaim() {
    let (mut node, runtime, _clock, grant, _owner, activation, _key, _remote) = fixture().await;
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let (session, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    let session = session.unwrap();
    let capacity = capacity.unwrap();
    let old_epoch = node.group.epoch();
    let joining_device = crate::MlsDevice::generate().unwrap();
    let package = joining_device.key_package().unwrap();
    node.with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert!(node.group.epoch() > old_epoch);
    assert!(!activation.is_active() && !session.is_current() && !capacity.is_current());
    assert_eq!(
        node.prepare_admitted_voice_admission_step(
            &grant,
            &activation,
            Some(&session.observer()),
            2,
            &payment
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    // A real old preclaim still owns its original slots and memory until Drop;
    // epoch invalidation cannot make its still-owned backing reusable early.
    assert_eq!(session.observer().unreserved_counts(), (20, 26, 26));
    drop(capacity);
    assert_eq!(session.observer().unreserved_counts(), (32, 32, 32));
}

#[tokio::test]
async fn admitted_neutral_only_preclaim_keeps_original_registration_and_pools_after_actual_root_return(
) {
    let (mut node, runtime, _clock, grant, _owner, activation, _key, _remote) = fixture().await;
    let payment = AdmissionRpcPermit::reserve(&runtime.resources(), grant.scope()).unwrap();
    let baseline = runtime.resources().counts();
    let (session, capacity) = node
        .prepare_admitted_voice_admission_step(&grant, &activation, None, 1, &payment)
        .unwrap()
        .into_parts();
    let session = session.unwrap();
    let capacity = capacity.unwrap();
    let (entered, resume) = capacity.pause_pool_return();
    drop(session); // No outer registration observer is kept by this fixture.
    node.clear_admitted_voice_call(&activation).unwrap();
    assert!(
        activation.root_returned().is_some(),
        "actual original Root residence must have returned"
    );
    assert!(!capacity.is_current());
    let actual = Producer::spawn(move || drop(capacity));
    bounded(entered).await.unwrap();
    // Do not synchronously read a potentially contested payer mutex on the
    // watchdog executor. RAII Resume still unpauses the worker on any failure.
    let sampler = runtime.resources();
    let during = Producer::spawn(move || sampler.counts()).join().await;
    drop(resume);
    actual.join().await;
    assert_eq!(
        during,
        (
            baseline.0 - WATCHER_BYTES + POOLS_BYTES + REGISTRATION_BYTES,
            baseline.1
        ),
        "the SAME pool and registration payments survive actual pool-body destruction"
    );
    assert_eq!(
        runtime.resources().counts(),
        (baseline.0 - WATCHER_BYTES - PRESENCE_BYTES, baseline.1),
        "actual preclaim return also releases the original Presence watcher custody"
    );
}
