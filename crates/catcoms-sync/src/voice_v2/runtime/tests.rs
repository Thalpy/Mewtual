use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::{ManualClock, RequestCancellation};
use std::future::Future;
use tokio::sync::watch;
mod local;

fn device(seed: u8) -> DeviceId {
    DeviceKeypair::from_seed(&[seed; 32]).device_id()
}

#[test]
fn voice_transport_maximum_serialized_verification_peak_stays_charged_until_last_owner() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = active(&runtime, 1);
    // Retained requester bookkeeping is a separate owner from the serialized verification
    // buffers. Install it before sampling so this regression measures only actual crypto work.
    drop(
        actor
            .admit_transport_verification(device(2), b"warm requester")
            .unwrap(),
    );
    let before = runtime.resources().counts();
    let bytes = vec![0; super::super::MAX_ENVELOPE];
    let ticket = actor
        .admit_transport_verification(device(2), &bytes)
        .unwrap();
    let work = ticket.actual_work();
    assert_eq!(
        runtime.resources().counts(),
        (before.0 + 5 * bytes.len() + 4096, before.1 + 1)
    );
    // This accounts serialized buffers, not allocator capacity/slack. The largest accepted
    // envelope reserves five simultaneous full-frame buffers plus bounded ACK metadata.
    drop(ticket);
    assert_eq!(runtime.resources().counts().1, before.1 + 1);
    drop(work);
    assert_eq!(runtime.resources().counts(), before);
}
fn view(epoch: u64, seeds: &[u8]) -> View {
    View {
        epoch,
        members: seeds.iter().map(|seed| device(*seed)).collect(),
        policy: Some([7; 32]),
        eligible: true,
        indirect: true,
    }
}
fn active(runtime: &Arc<Runtime>, epoch: u64) -> ActorOwner {
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    registration.register().unwrap();
    actor.sync_view(view(epoch, &[1, 2, 3])).unwrap();
    actor
}

#[test]
fn response_member_view_borrows_original_map_and_rejects_stale_scope_epoch_policy_and_session() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let mut actor = active(&runtime, 1);
    let original = actor
        .source_context(super::super::Scope {
            group: [1; 32],
            policy: Some([7; 32]),
            room: 9,
        })
        .unwrap();
    let before = runtime.resources().counts();
    assert_eq!(
        actor.with_current_members(&original, |member| {
            assert!(member(device(1)) && member(device(2)) && member(device(3)));
            assert!(!member(device(4)));
            // The pure callback borrows the SAME map under its actual native fence.
            assert!(actor.group.state.try_lock().is_err());
            Ok(())
        }),
        Ok(())
    );
    assert_eq!(runtime.resources().counts(), before);
    for altered in [
        super::super::Context {
            epoch: 2,
            ..original
        },
        super::super::Context {
            local: device(2),
            ..original
        },
        super::super::Context {
            scope: super::super::Scope {
                group: [2; 32],
                ..original.scope
            },
            ..original
        },
        super::super::Context {
            scope: super::super::Scope {
                policy: None,
                ..original.scope
            },
            ..original
        },
        super::super::Context {
            indirect: false,
            ..original
        },
    ] {
        assert_eq!(
            actor.with_current_members(&altered, |_| -> Result<(), VoiceError> {
                panic!("stale native view must refuse before the role callback")
            }),
            Err(VoiceError::Unauthorized)
        );
    }
    actor.sync_view(view(2, &[1, 3])).unwrap();
    assert_eq!(
        actor.with_current_members(&original, |_| Ok(())),
        Err(VoiceError::Unauthorized)
    );
    let current = super::super::Context {
        epoch: 2,
        ..original
    };
    assert_eq!(
        actor.with_current_members(&current, |member| {
            assert!(!member(device(2)) && member(device(3)));
            Ok(())
        }),
        Ok(())
    );
    runtime.revoke_session();
    assert_eq!(
        actor.with_current_members(&current, |_| Ok(())),
        Err(VoiceError::Unauthorized)
    );
}

#[test]
fn collection_releases_cold_locked_group_origins_without_erasing_membership_evidence() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let mut actors = Vec::new();
    for group in 1..=8 {
        let (mut actor, registration) = runtime
            .prepare_actor([group; 32], device(1), &ticket)
            .unwrap();
        registration.register().unwrap();
        actor
            .sync_view(view(7, &(1..=64).collect::<Vec<_>>()))
            .unwrap();
        actor
            .with_limits(|limits| {
                for member in 1..=64 {
                    limits.requester(device(member), 1000)?;
                }
                Ok(())
            })
            .unwrap();
        actors.push(actor);
    }
    let (mut fresh, registration) = runtime.prepare_actor([9; 32], device(1), &ticket).unwrap();
    registration.register().unwrap();
    fresh.sync_view(view(7, &[1, 2, 3])).unwrap();
    assert_eq!(
        fresh.with_limits(|limits| limits.requester(device(2), 1000)),
        Err(VoiceError::Capacity)
    );
    let first = actors[0].group.clone();
    let revision = first.state.lock().unwrap().revision;
    let retained_member = first.state.lock().unwrap().members[&device(2)].clone();
    runtime.revoke_session();
    drop(actors);
    drop(fresh);
    clock.advance_ms(60_000);
    for _ in 0..3 {
        runtime.collect_expired();
    }
    // The old groups were never touched through a new actor or packet. Global collection alone
    // frees process Origin slots, while same-epoch protection remains identical, not recreated.
    {
        let state = first.state.lock().unwrap();
        assert_eq!(state.epoch, Some(7));
        assert_eq!(state.revision, revision);
        assert!(Arc::ptr_eq(&state.members[&device(2)], &retained_member));
    }
    clock.set_wall_ms(1000);
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut fresh, registration) = runtime.prepare_actor([9; 32], device(1), &ticket).unwrap();
    registration.register().unwrap();
    fresh.sync_view(view(7, &[1, 2, 3])).unwrap();
    fresh
        .with_limits(|limits| limits.requester(device(2), clock.monotonic_ms()))
        .unwrap();
    let (mut stale, registration) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    registration.register().unwrap();
    assert!(stale.sync_view(view(7, &[1, 2, 3])).is_err());
}

#[test]
fn prepared_actor_needs_both_native_registration_and_exact_unlock_commit() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock);
    let ticket = runtime.session_ticket();
    let (mut actor, registration) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    assert_eq!(
        actor.sync_view(view(1, &[1, 2])).err(),
        Some(VoiceError::Unauthorized)
    );
    registration.register().unwrap();
    assert_eq!(
        actor.sync_view(view(1, &[1, 2])).err(),
        Some(VoiceError::Unauthorized)
    );
    runtime.activate_session(&ticket).unwrap();
    actor.sync_view(view(1, &[1, 2])).unwrap();
    assert!(actor.child_lease().unwrap().is_live());
}

#[test]
fn dropping_process_registry_revokes_retained_actors_children_and_effects() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = active(&runtime, 1);
    let child = actor.child_lease().unwrap();
    let watcher = actor.watch_lease().unwrap();
    let effect = actor.effect(device(2), device(3), 16_000).unwrap();
    let ticket = runtime.session_ticket();
    let (_candidate, registration) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    let resources = runtime.resources();
    let before = resources.counts();
    drop(runtime);
    assert!(!effect.authorized() && !child.is_live() && !watcher.is_live());
    assert!(actor.child_lease().is_err() && registration.register().is_err());
    assert_eq!(
        resources.counts(),
        before,
        "old actual holders keep the old pool charged"
    );
}

#[test]
fn native_install_distinguishes_optional_expiry_from_competing_actor_revision() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let ticket = runtime.session_ticket();
    let (_expired, expired) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    clock.advance_ms(PREPARATION_MS);
    assert_eq!(
        expired.register_install(),
        Err(RegistrationError::Unavailable(VoiceError::Unauthorized))
    );
    let (_first, first) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    let (_second, second) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    first.register_install().unwrap();
    clock.advance_ms(PREPARATION_MS);
    assert_eq!(
        second.register_install(),
        Err(RegistrationError::Conflict),
        "expiry cannot hide a genuine competing install"
    );
    assert!(first.is_current_for(&ticket));
}

#[test]
fn contradictory_current_actor_view_revokes_existing_effects_and_children() {
    for contradiction in [view(0, &[1, 2, 3]), view(1, &[1, 3])] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let mut actor = active(&runtime, 1);
        let child = actor.child_lease().unwrap();
        let effect = actor.effect(device(2), device(3), 16_000).unwrap();
        assert_eq!(
            actor.sync_view(contradiction).err(),
            Some(VoiceError::Freshness)
        );
        assert!(!child.is_live() && !effect.authorized());
        assert_eq!(
            actor.sync_view(view(2, &[1, 2, 3])).err(),
            Some(VoiceError::Unauthorized)
        );
    }
}

#[test]
fn old_unlock_and_prepared_actor_cannot_activate_after_lock() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    let (mut old, registration) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    runtime.revoke_session();
    assert_eq!(
        runtime.activate_session(&ticket).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        registration.register().err(),
        Some(VoiceError::Unauthorized)
    );
    let new_ticket = runtime.session_ticket();
    runtime.activate_session(&new_ticket).unwrap();
    assert_eq!(
        old.sync_view(view(1, &[1, 2])).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(runtime.prepare_actor([1; 32], device(1), &ticket).is_err());
}

#[test]
fn native_registration_cas_retires_old_actor_before_any_new_activation() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let old = active(&runtime, 1);
    let old_child = old.child_lease().unwrap();
    let ticket = runtime.session_ticket();
    let (mut first, first_registration) =
        runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    let (mut stale, stale_registration) =
        runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    first_registration.register().unwrap();
    assert!(
        !old_child.is_live(),
        "successful native replacement retires the old root immediately"
    );
    assert_eq!(
        stale_registration.register().err(),
        Some(VoiceError::Unauthorized)
    );
    first.sync_view(view(1, &[1, 2, 3])).unwrap();
    let winner = first.child_lease().unwrap();
    assert!(!old_child.is_live());
    assert!(old.child_lease().is_err());
    assert_eq!(
        stale.sync_view(view(1, &[1, 2, 3])).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(winner.is_live());
}

#[test]
fn installed_winner_is_independent_of_deferred_actor_activation_order() {
    for winner_first in [false, true] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let old = active(&runtime, 1);
        let old_child = old.child_lease().unwrap();
        let ticket = runtime.session_ticket();
        let (mut a, a_install) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
        a_install.register().unwrap(); // Native registry now installs A; A has not run its turn.
        let (mut b, b_install) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
        b_install.register().unwrap(); // A later native installation supersedes pending A.
        assert!(!old_child.is_live());
        if winner_first {
            b.sync_view(view(1, &[1, 2, 3])).unwrap();
        }
        assert_eq!(
            a.sync_view(view(1, &[1, 2, 3])).err(),
            Some(VoiceError::Unauthorized)
        );
        if !winner_first {
            b.sync_view(view(1, &[1, 2, 3])).unwrap();
        }
        let winner = b.child_lease().unwrap();
        assert!(a.child_lease().is_err() && old.child_lease().is_err());
        assert_eq!(a_install.register().err(), Some(VoiceError::Unauthorized));
        b_install.register().unwrap(); // Exact winner registration is idempotent.
        assert!(winner.is_live());
    }
}

#[test]
fn registered_inactive_restores_do_not_consume_uncommitted_preparation_slots() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let ticket = runtime.session_ticket();
    let mut actors = Vec::new();
    for group in 1..=12 {
        let (mut actor, installation) = runtime
            .prepare_actor([group; 32], device(1), &ticket)
            .unwrap();
        installation.register().unwrap();
        assert_eq!(
            actor.sync_view(view(1, &[1, 2, 3])).err(),
            Some(VoiceError::Unauthorized)
        );
        actors.push(actor);
    }
    runtime.activate_session(&ticket).unwrap();
    for actor in &mut actors {
        actor.sync_view(view(1, &[1, 2, 3])).unwrap();
        assert!(actor.child_lease().unwrap().is_live());
    }
}

#[test]
fn abandoned_or_expired_preparations_revoke_without_refunding_actual_retained_owners() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let _old = active(&runtime, 1);
    let ticket = runtime.session_ticket();
    let (mut candidate, installation) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    let before = runtime.resources().counts();
    let last_waiter = installation.clone();
    drop(installation);
    drop(last_waiter);
    assert_eq!(
        candidate.sync_view(view(1, &[1, 2, 3])).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(runtime.resources().counts(), before);
    let (_next, installation) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    clock.advance_ms(PREPARATION_MS);
    assert_eq!(
        installation.register().err(),
        Some(VoiceError::Unauthorized)
    );
    // Revoked-but-held actual owner charges survive; they do not prevent fresh preparation.
    runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
}

#[test]
fn registered_stale_snapshot_waits_for_control_without_authorizing_itself_or_old_actor() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let mut old = active(&runtime, 10);
    old.sync_view(view(11, &[1, 2])).unwrap();
    let old_lease = old.child_lease().unwrap();
    let ticket = runtime.session_ticket();
    let (mut candidate, registration) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    assert!(
        old_lease.is_live(),
        "preparation alone cannot retire the installed actor"
    );
    registration.register().unwrap();
    assert!(
        !old_lease.is_live(),
        "native installation retires old authority even before control catches up"
    );
    assert_eq!(
        candidate.sync_view(view(10, &[1, 2, 3])).err(),
        Some(VoiceError::Freshness)
    );
    assert_eq!(
        candidate.sync_view(view(11, &[1, 2, 3])).err(),
        Some(VoiceError::Freshness)
    );
    assert!(candidate.child_lease().is_err());
    candidate.sync_view(view(11, &[1, 2])).unwrap();
    assert!(!old_lease.is_live());
}

#[tokio::test]
async fn queued_effect_rejects_removed_and_readmitted_origin_or_final_member() {
    for removed in [2, 3] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let mut actor = active(&runtime, 1);
        let effect = actor.effect(device(2), device(3), 16_000).unwrap();
        let (_cancel, receiver) = watch::channel(false);
        let mut queued = RequestCancellation::new(receiver, None).with_authority(effect.clone());
        assert!(!queued.is_cancelled());
        let remaining = if removed == 2 { vec![1, 3] } else { vec![1, 2] };
        actor.sync_view(view(2, &remaining)).unwrap();
        actor.sync_view(view(3, &[1, 2, 3])).unwrap();
        assert!(
            queued.is_cancelled(),
            "a still-current helper cannot revive an old member generation"
        );
        queued.cancelled().await;
        assert!(!effect.authorized());
        assert!(actor
            .effect(device(2), device(3), 16_000)
            .unwrap()
            .authorized());
    }
}

#[tokio::test]
async fn exact_effect_wait_wakes_after_registration_and_never_refunds_retained_custody() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let mut actor = active(&runtime, 1);
    let effect = actor.effect(device(2), device(3), 16_000).unwrap();
    let before = runtime.resources().counts();
    {
        let mut waiting = std::pin::pin!(effect.revoked());
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        actor.sync_view(view(2, &[1, 3])).unwrap();
        waiting.await;
    }
    // A detached lower holder still owns the old member and actor credits after logical revoke.
    assert_eq!(runtime.resources().counts(), before);
    drop(effect);
    assert_eq!(runtime.resources().counts().0, before.0 - 128);
}

#[test]
fn lock_and_policy_changes_revoke_children_and_detached_effects() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let mut actor = active(&runtime, 1);
    let child = actor.child_lease().unwrap();
    let watch = actor.watch_lease().unwrap();
    let effect = actor.effect(device(2), device(3), 16_000).unwrap();
    let mut changed = view(2, &[1, 2, 3]);
    changed.policy = Some([8; 32]);
    actor.sync_view(changed).unwrap();
    assert!(!child.is_live() && !watch.is_live() && !effect.authorized());
    let new = actor.child_lease().unwrap();
    runtime.revoke_session();
    assert!(!new.is_live());
    let _replacement = active(&runtime, 2);
    assert!(!effect.authorized() && !new.is_live());
}

#[test]
fn ineligible_policy_and_actor_drop_admit_no_effect_or_participant_authority() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let mut actor = active(&runtime, 1);
    let effect = actor.effect(device(2), device(3), 16_000).unwrap();
    let mut changed = view(2, &[1, 2, 3]);
    changed.eligible = false;
    actor.sync_view(changed).unwrap();
    assert!(!effect.authorized());
    assert!(actor.child_lease().is_err() && actor.watch_lease().is_err());
    actor.sync_view(view(3, &[1, 2, 3])).unwrap();
    let new = actor.effect(device(2), device(3), 16_000).unwrap();
    drop(actor);
    assert!(!new.authorized());
}

#[test]
fn rates_and_binding_protection_survive_actor_and_session_replacement() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock);
    let actor = active(&runtime, 1);
    for _ in 0..40 {
        actor
            .with_limits(|limits| limits.requester(device(2), 1000))
            .unwrap();
    }
    actor
        .with_limits(|limits| limits.join(device(2), [2; 32], [3; 32], 1000, 41_000))
        .unwrap();
    runtime.revoke_session();
    let new = active(&runtime, 1);
    assert_eq!(
        new.with_limits(|limits| limits.requester(device(2), 1000))
            .err(),
        Some(VoiceError::RateLimited)
    );
    assert_eq!(
        new.with_limits(|limits| limits.join(device(2), [2; 32], [4; 32], 1000, 41_000))
            .err(),
        Some(VoiceError::Unauthorized)
    );
}

#[test]
fn effective_time_never_resurrects_expired_objects_after_forward_step_and_rollback() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    let effect = actor.effect(device(2), device(3), 16_000).unwrap();
    clock.set_wall_ms(100_000);
    assert!(!effect.authorized());
    clock.set_wall_ms(1000);
    assert_eq!(runtime.clock().now_ms(), 100_000);
    assert!(!effect.authorized());
    assert_eq!(
        actor
            .effect(device(2), device(3), 16_000)
            .err()
            .map(|_| VoiceError::Freshness),
        Some(VoiceError::Freshness)
    );
}

#[test]
fn signed_time_keeps_advancing_when_wall_rolls_back_before_expiry() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    let effect = actor.effect(device(2), device(3), 16_000).unwrap();
    clock.advance_ms(61_000);
    clock.set_wall_ms(500);
    assert_eq!(runtime.clock().now_ms(), 62_000);
    assert!(!effect.authorized());
    assert_eq!(
        actor.effect(device(2), device(3), 16_000).err(),
        Some(VoiceError::Freshness)
    );
    // Repeated observations/concurrent elapsed snapshots cannot count the same interval twice.
    assert_eq!(runtime.clock().now_ms(), 62_000);
    clock.advance_ms(1000);
    clock.set_wall_ms(500);
    assert_eq!(runtime.clock().now_ms(), 63_000);
}

#[test]
fn prepared_and_child_caps_count_actual_owners_instead_of_lost_waiters() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = active(&runtime, 1);
    let ticket = runtime.session_ticket();
    let mut held = Vec::new();
    for _ in 0..PREPARED_ACTORS {
        held.push(runtime.prepare_actor([1; 32], device(1), &ticket).unwrap());
    }
    assert_eq!(
        runtime.prepare_actor([1; 32], device(1), &ticket).err(),
        Some(VoiceError::Capacity)
    );
    held.pop();
    runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
    let children = (0..CHILDREN)
        .map(|_| actor.child_lease().unwrap())
        .collect::<Vec<_>>();
    assert!(actor.child_lease().is_err());
    children[0].revoke();
    actor.child_lease().unwrap();
}

#[derive(Debug)]
struct RegisterObservedClock {
    inner: ManualClock,
    observed: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    revoke_on_wall: Mutex<Option<CallLease>>,
}
#[test]
fn endpoint_final_native_time_sample_rechecks_parent_before_spending_origin_allowance() {
    use super::super::{Body, Context, Envelope, JoinKey, Scope, SignedObject};
    let clock = Arc::new(RegisterObservedClock {
        inner: ManualClock::new(1000),
        observed: Mutex::new(None),
        revoke_on_wall: Mutex::new(None),
    });
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    let context = Context {
        scope: Scope {
            group: [1; 32],
            policy: Some([7; 32]),
            room: 1,
        },
        local: device(1),
        now: 1000,
        epoch: 1,
        indirect: true,
    };
    let signer = DeviceKeypair::from_seed(&[2; 32]);
    let key = JoinKey::new([2; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap();
    let envelope = |id| {
        Envelope::Direct(
            SignedObject::issue(
                context.scope,
                &signer,
                [id; 32],
                1000,
                31_000,
                Body::Announcement {
                    key: key.clone(),
                    epoch: 1,
                    renewal: 1,
                },
            )
            .unwrap(),
        )
    };
    let parent = CallLease::default();
    let watch = actor
        .watch_lease()
        .unwrap()
        .with_parent(parent.clone())
        .unwrap();
    let original = envelope(51);
    let ticket = actor
        .admit_transport_verification(device(2), &original.encode())
        .unwrap();
    actor
        .authorize_verified_route(&ticket, &original, &context)
        .unwrap();
    // Native End wins during the final transaction's clock observation, after preparation and
    // before origin commit. This is an atomic consent close, not reentry into runtime locks.
    *clock.revoke_on_wall.lock().unwrap() = Some(parent);
    assert_eq!(
        actor.commit_endpoint_origin(&ticket, &original, &context, |_, _| {
            if watch.is_live() {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        }),
        Err(VoiceError::Unauthorized)
    );
    assert!(!watch.is_live());
    let current = actor.watch_lease().unwrap();
    // An abandoned affine origin transaction has no quota effect. A protected row may
    // refuse here before it consumes origin, and must then return its unpublished claim.
    let untouched = envelope(61);
    let untouched_ticket = actor
        .admit_transport_verification(device(2), &untouched.encode())
        .unwrap();
    assert_eq!(
        actor.with_endpoint_origin(&untouched_ticket, &untouched, &context, |_, _, _origin| {
            Err::<(), _>(VoiceError::Capacity)
        }),
        Err(VoiceError::Capacity)
    );
    // This unit exercises only the final budget transaction. Root intake tests separately pin
    // persistent queue reservation. Rejection above must leave all eight original room tokens.
    for id in 52..=60 {
        let original = envelope(id);
        let ticket = actor
            .admit_transport_verification(device(2), &original.encode())
            .unwrap();
        let result = actor.commit_endpoint_origin(&ticket, &original, &context, |_, _| {
            if current.is_live() {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        });
        assert_eq!(
            result,
            if id < 60 {
                Ok(())
            } else {
                Err(VoiceError::RateLimited)
            }
        );
    }
    let refused = envelope(62);
    let refused_ticket = actor
        .admit_transport_verification(device(2), &refused.encode())
        .unwrap();
    let refused_result: Result<(), VoiceError> =
        actor.with_endpoint_origin(&refused_ticket, &refused, &context, |_, _, origin| {
            origin.publish(|| panic!("origin refusal cannot publish a protected row"))
        });
    assert_eq!(refused_result, Err(VoiceError::RateLimited));
}
#[test]
fn endpoint_origin_loan_rechecks_session_actor_and_actual_driver_before_publication() {
    use super::super::{Body, Context, Envelope, JoinKey, Scope, SignedObject};
    for revoke in ["session", "actor", "driver"] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let actor = active(&runtime, 1);
        let context = Context {
            scope: Scope {
                group: [1; 32],
                policy: Some([7; 32]),
                room: 1,
            },
            local: device(1),
            now: 1000,
            epoch: 1,
            indirect: true,
        };
        let point = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
        let key = JoinKey::new([2; 32], point.try_into().unwrap()).unwrap();
        let signer = DeviceKeypair::from_seed(&[2; 32]);
        let envelope = Envelope::Direct(
            SignedObject::issue(
                context.scope,
                &signer,
                [51; 32],
                1000,
                31_000,
                Body::Announcement {
                    key,
                    epoch: 1,
                    renewal: 1,
                },
            )
            .unwrap(),
        );
        let (responder, receiver) = catcoms_rt::Responder::channel();
        let mut receiver = Some(receiver);
        let ticket = actor
            .admit_transport_verification(device(2), &envelope.encode())
            .unwrap()
            .bind_request(responder.cancellation_authority())
            .unwrap();
        let watch = actor.watch_lease().unwrap();
        let mut published = false;
        let result = actor.with_endpoint_origin(&ticket, &envelope, &context, |_, _, origin| {
            // This models the first atomic part of lock/replacement while its policy/child
            // cleanup would wait for the very Policy mutex held by this transaction.
            match revoke {
                "session" => actor.gate.session.live.revoke(),
                "actor" => actor.gate.live.revoke(),
                "driver" => drop(receiver.take().unwrap()),
                _ => unreachable!(),
            }
            assert!(
                watch.is_live(),
                "old child cleanup has not acquired Policy yet"
            );
            origin.publish(|| {
                published = true;
            })
        });
        assert_eq!(result, Err(VoiceError::Unauthorized), "{revoke}");
        assert!(!published, "{revoke} must refuse before publication");
        // Probe quota directly: this old actor is rightly ineligible for a fresh operation.
        // All eight original room tokens must remain after the refused final origin commit.
        let mut state = actor.group.state.lock().unwrap();
        for _ in 0..8 {
            state
                .limits
                .origin(device(2), super::super::limits::Traffic::Room, 0)
                .unwrap();
        }
        assert_eq!(
            state
                .limits
                .origin(device(2), super::super::limits::Traffic::Room, 0),
            Err(VoiceError::RateLimited)
        );
    }
}

/// Observe the real final admission locks through injected Clock calls. This detects lock
/// reentry without blocking the test on the very defect it is meant to demonstrate.
struct OriginClockProbe {
    inner: ManualClock,
    owners: Mutex<Option<(Weak<Group>, Weak<ActorGate>)>>,
    outside: AtomicBool,
    samples: AtomicU64,
}
redacted_debug!(OriginClockProbe);
impl OriginClockProbe {
    fn observe(&self) {
        if let Some((group, actor)) = self.owners.lock().unwrap().as_ref() {
            let group = group.upgrade().unwrap();
            let actor = actor.upgrade().unwrap();
            self.outside.fetch_and(
                group.state.try_lock().is_ok() && actor.policy.try_lock().is_ok(),
                Ordering::AcqRel,
            );
            self.samples.fetch_add(1, Ordering::Relaxed);
        }
    }
}
impl Clock for OriginClockProbe {
    fn now_ms(&self) -> u64 {
        self.observe();
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        self.observe();
        self.inner.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}

fn endpoint_origin_input(
    actor: &ActorOwner,
) -> (super::super::Context, super::super::Envelope, IngressTicket) {
    use super::super::{Body, Context, Envelope, JoinKey, Scope, SignedObject};
    let context = Context {
        scope: Scope {
            group: [1; 32],
            policy: Some([7; 32]),
            room: 1,
        },
        local: device(1),
        now: 1000,
        epoch: 1,
        indirect: true,
    };
    let signer = DeviceKeypair::from_seed(&[2; 32]);
    let point = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    let envelope = Envelope::Direct(
        SignedObject::issue(
            context.scope,
            &signer,
            [81; 32],
            1000,
            31_000,
            Body::Announcement {
                key: JoinKey::new([2; 32], point.try_into().unwrap()).unwrap(),
                epoch: 1,
                renewal: 1,
            },
        )
        .unwrap(),
    );
    let ticket = actor
        .admit_transport_verification(device(2), &envelope.encode())
        .unwrap();
    (context, envelope, ticket)
}

#[test]
fn endpoint_origin_samples_time_without_holding_group_or_policy() {
    let clock = Arc::new(OriginClockProbe {
        inner: ManualClock::new(1000),
        owners: Mutex::new(None),
        outside: AtomicBool::new(true),
        samples: AtomicU64::new(0),
    });
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    let (context, envelope, ticket) = endpoint_origin_input(&actor);
    *clock.owners.lock().unwrap() =
        Some((Arc::downgrade(&actor.group), Arc::downgrade(&actor.gate)));
    actor
        .with_endpoint_origin(&ticket, &envelope, &context, |_, _, origin| {
            origin.publish(|| ())
        })
        .unwrap();
    assert!(clock.samples.load(Ordering::Relaxed) > 0);
    assert!(
        clock.outside.load(Ordering::Acquire),
        "final origin admission must not call the injected Clock while Group or Policy is held"
    );
}

#[test]
fn voice_transport_verification_samples_before_native_guards_and_refuses_busy_view_without_work() {
    let clock = Arc::new(OriginClockProbe {
        inner: ManualClock::new(1000),
        owners: Mutex::new(None),
        outside: AtomicBool::new(true),
        samples: AtomicU64::new(0),
    });
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    *clock.owners.lock().unwrap() =
        Some((Arc::downgrade(&actor.group), Arc::downgrade(&actor.gate)));
    let before = runtime.resources().counts();
    {
        let _group = actor.group.state.lock().unwrap();
        assert_eq!(
            actor
                .admit_transport_verification(device(2), b"bounded frame")
                .err(),
            Some(VoiceError::Capacity)
        );
    }
    {
        let _policy = actor.gate.policy.lock().unwrap();
        assert_eq!(
            actor
                .admit_transport_verification(device(2), b"bounded frame")
                .err(),
            Some(VoiceError::Capacity)
        );
    }
    assert_eq!(runtime.resources().counts(), before);
    // The deliberate externally-held guards above aren't Runtime-held samples. Now check
    // the actual successful path, including EffectiveClock's own source-clock callbacks.
    clock.outside.store(true, Ordering::Release);
    clock.inner.advance_ms(700);
    let ticket = actor
        .admit_transport_verification(device(2), b"bounded frame")
        .unwrap();
    assert!(clock.outside.load(Ordering::Acquire));
    assert!(clock.samples.load(Ordering::Relaxed) > 0);
    assert_eq!(ticket.deadline, 6700, "retry obtains its own fresh5s bound");
    clock.inner.advance_ms(5000);
    assert!(ticket.check(&actor, b"bounded frame").is_err());
}

#[test]
fn endpoint_origin_contention_is_retryable_without_origin_debit_or_stale_time() {
    for policy_busy in [false, true] {
        let clock = Arc::new(ManualClock::new(1000));
        let runtime = Runtime::new(clock.clone());
        let actor = active(&runtime, 1);
        let (context, envelope, ticket) = endpoint_origin_input(&actor);
        let attempt = || {
            actor.with_endpoint_origin(&ticket, &envelope, &context, |_, _, origin| {
                origin.publish(|| ())
            })
        };
        // The test thread itself owns the contended mutex. A blocking implementation would
        // deadlock here instead of returning an effect-free, retryable capacity refusal.
        if policy_busy {
            let _held = actor.gate.policy.lock().unwrap();
            assert_eq!(attempt(), Err(VoiceError::Capacity));
        } else {
            let _held = actor.group.state.lock().unwrap();
            assert_eq!(attempt(), Err(VoiceError::Capacity));
        }
        // The failed turn must not freeze its old clock samples for a later successful turn.
        clock.advance_ms(60_000);
        assert_eq!(attempt(), Err(VoiceError::Freshness));
        let mut state = actor.group.state.lock().unwrap();
        for _ in 0..8 {
            state
                .limits
                .origin(
                    device(2),
                    super::super::limits::Traffic::Room,
                    clock.monotonic_ms(),
                )
                .unwrap();
        }
        assert_eq!(
            state.limits.origin(
                device(2),
                super::super::limits::Traffic::Room,
                clock.monotonic_ms()
            ),
            Err(VoiceError::RateLimited)
        );
    }
}

impl Clock for RegisterObservedClock {
    fn now_ms(&self) -> u64 {
        if let Some(lease) = self.revoke_on_wall.lock().unwrap().take() {
            lease.revoke();
        }
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        let now = self.inner.monotonic_ms();
        if let Some(sender) = self.observed.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
        now
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}
#[test]
fn registration_rechecks_dropped_or_expired_candidate_after_waiting_for_group_cas() {
    for drop_owner in [false, true] {
        let clock = Arc::new(RegisterObservedClock {
            inner: ManualClock::new(1000),
            observed: Mutex::new(None),
            revoke_on_wall: Mutex::new(None),
        });
        let runtime = Runtime::new(clock.clone());
        let old = active(&runtime, 1);
        let old_child = old.child_lease().unwrap();
        let old_effect = old.effect(device(2), device(3), 16_000).unwrap();
        let ticket = runtime.session_ticket();
        let (candidate, installation) = runtime.prepare_actor([1; 32], device(1), &ticket).unwrap();
        let group = old.group.clone();
        let held = group.state.lock().unwrap();
        let (entered, seen) = std::sync::mpsc::channel();
        *clock.observed.lock().unwrap() = Some(entered);
        let worker = std::thread::spawn(move || installation.register());
        // The first TTL read occurred, but registration cannot pass the held CAS mutex yet.
        seen.recv().unwrap();
        let mut candidate = Some(candidate);
        if drop_owner {
            candidate.take();
        } else {
            clock.inner.advance_ms(PREPARATION_MS);
        }
        drop(held);
        assert_eq!(worker.join().unwrap(), Err(VoiceError::Unauthorized));
        assert!(old_child.is_live());
        if drop_owner {
            assert!(old_effect.authorized());
        }
        // In the TTL case the old original packet naturally expired; its actor still owns calls.
        assert!(old.child_lease().unwrap().is_live());
        drop(candidate);
    }
}

#[test]
fn final_origin_commit_rechecks_session_revoked_during_native_time_sample() {
    use super::super::{Body, Envelope, JoinKey, RoutePermit, Scope, SignedObject};
    let clock = Arc::new(RegisterObservedClock {
        inner: ManualClock::new(1000),
        observed: Mutex::new(None),
        revoke_on_wall: Mutex::new(None),
    });
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    let context = super::super::Context {
        scope: Scope {
            group: [1; 32],
            policy: Some([7; 32]),
            room: 12,
        },
        local: device(1),
        now: 1000,
        epoch: 1,
        indirect: true,
    };
    let public = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    let key = JoinKey::new([2; 32], public.try_into().unwrap()).unwrap();
    let join = key.join;
    let signer = DeviceKeypair::from_seed(&[2; 32]);
    let object = SignedObject::issue(
        context.scope,
        &signer,
        [3; 32],
        1000,
        31_000,
        Body::Announcement {
            key,
            epoch: 1,
            renewal: 1,
        },
    )
    .unwrap();
    let permit = RoutePermit::issue(&signer, &object, device(1), 1000, 16_000).unwrap();
    let envelope = Envelope::ToHelper { object, permit };
    actor
        .authorize_route(device(2), &envelope, &context)
        .unwrap();
    let helper = actor.helper().unwrap();
    let before = runtime.resources().counts();
    // Match lock's first irreversible revocation, before it can acquire the Policy mutex. The
    // early current check passed, but the final native-time sample closes the same session.
    *clock.revoke_on_wall.lock().unwrap() = Some(actor.gate.session.live.clone());
    assert_eq!(
        actor.commit_route_origin(device(2), &envelope, &context, &CallLease::default()),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(helper.counts(), (0, 0, 0));
    assert_eq!(runtime.resources().counts(), before);
    let mut state = actor.group.state.lock().unwrap();
    assert_eq!(state.limits.delegation_deadline(device(2), join), None);
    for _ in 0..8 {
        state
            .limits
            .origin(device(2), super::super::limits::Traffic::Room, 0)
            .unwrap();
    }
    assert_eq!(
        state
            .limits
            .origin(device(2), super::super::limits::Traffic::Room, 0),
        Err(VoiceError::RateLimited)
    );
}
