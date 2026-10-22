use super::*;

fn context() -> crate::voice_v2::Context {
    crate::voice_v2::Context {
        scope: crate::voice_v2::Scope {
            group: [1; 32],
            policy: Some([7; 32]),
            room: 1,
        },
        local: device(1),
        now: 1000,
        epoch: 1,
        indirect: true,
    }
}

#[test]
fn local_origin_and_effect_sample_clock_outside_guards_and_refuse_contention() {
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
    let before = clock.samples.load(Ordering::Acquire);
    {
        let _group = actor.group.state.lock().unwrap();
        assert_eq!(
            actor.with_local_origin(device(2), &context(), |_, _, _| panic!("busy group")),
            Err(VoiceError::Capacity)
        );
        assert_eq!(
            actor.effect(device(1), device(2), 16_000).err(),
            Some(VoiceError::Capacity)
        );
    }
    {
        let _policy = actor.gate.policy.lock().unwrap();
        assert_eq!(
            actor.with_local_origin(device(2), &context(), |_, _, _| panic!("busy policy")),
            Err(VoiceError::Capacity)
        );
        assert_eq!(
            actor.effect(device(1), device(2), 16_000).err(),
            Some(VoiceError::Capacity)
        );
    }
    assert!(
        clock.samples.load(Ordering::Acquire) > before,
        "every retry samples current time"
    );
    // The deliberate contention above holds guards in this test's own thread. Only successful
    // acquisition below establishes that the runtime itself never samples under either guard.
    clock.outside.store(true, Ordering::Release);
    clock.inner.advance_ms(700);
    actor
        .with_local_origin(device(2), &context(), |wall, mono, origin| {
            assert_eq!(wall, 1700);
            assert_eq!(mono, clock.inner.monotonic_ms());
            origin.publish(|| ())
        })
        .unwrap();
    let effect = actor.effect(device(1), device(2), 16_000).unwrap();
    assert!(effect.authorized());
    assert!(clock.outside.load(Ordering::Acquire));
}

#[test]
fn local_origin_rechecks_atomic_actor_and_session_close_before_publication_without_quota() {
    for session in [false, true] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let actor = active(&runtime, 1);
        let mut published = false;
        let result = actor.with_local_origin(device(2), &context(), |_, _, origin| {
            if session {
                actor.gate.session.live.revoke();
            } else {
                actor.gate.live.revoke();
            }
            origin.publish(|| {
                published = true;
            })
        });
        assert_eq!(result, Err(VoiceError::Unauthorized));
        assert!(!published);
        let mut state = actor.group.state.lock().unwrap();
        for _ in 0..8 {
            state
                .limits
                .origin(device(2), crate::voice_v2::limits::Traffic::Room, 0)
                .unwrap();
        }
        assert_eq!(
            state
                .limits
                .origin(device(2), crate::voice_v2::limits::Traffic::Room, 0),
            Err(VoiceError::RateLimited)
        );
    }
}

#[test]
fn local_origin_rejects_stale_epoch_removed_members_and_wrong_scope_before_callback() {
    for kind in ["epoch", "peer", "local", "policy", "ineligible", "self"] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let mut actor = active(&runtime, 1);
        let mut next = view(1, &[1, 2, 3]);
        let mut observed = context();
        let mut peer = device(2);
        match kind {
            "epoch" => next.epoch = 2,
            "peer" => {
                next.epoch = 2;
                observed.epoch = 2;
                next.members.remove(&device(2));
            }
            "local" => {
                next.epoch = 2;
                observed.epoch = 2;
                next.members.remove(&device(1));
            }
            "policy" => observed.scope.policy = Some([9; 32]),
            "ineligible" => next.eligible = false,
            "self" => peer = device(1),
            _ => unreachable!(),
        }
        if kind == "local" {
            assert_eq!(
                actor.sync_view(next),
                Err(VoiceError::Unauthorized),
                "a native view without the local member revokes the actor itself"
            );
        } else {
            actor.sync_view(next).unwrap();
        }
        assert_eq!(
            actor.with_local_origin(peer, &observed, |_, _, _| panic!("{kind}")),
            Err(VoiceError::Unauthorized),
            "{kind}"
        );
    }
}

#[test]
fn effect_last_clock_callback_cannot_issue_an_owner_after_atomic_actor_close() {
    let clock = Arc::new(RegisterObservedClock {
        inner: ManualClock::new(1000),
        observed: Mutex::new(None),
        revoke_on_wall: Mutex::new(None),
    });
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    *clock.revoke_on_wall.lock().unwrap() = Some(actor.gate.live.clone());
    assert_eq!(
        actor.effect(device(1), device(2), 16_000).err(),
        Some(VoiceError::Unauthorized)
    );
}

#[test]
fn source_context_samples_outside_native_guards_and_refuses_contention() {
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
    let observed = actor.source_context(context().scope).unwrap();
    assert_eq!((observed.now, observed.epoch), (1000, 1));
    assert!(clock.outside.load(Ordering::Acquire));
    {
        let _group = actor.group.state.lock().unwrap();
        assert_eq!(
            actor.source_context(context().scope).err(),
            Some(VoiceError::Capacity)
        );
    }
    {
        let _policy = actor.gate.policy.lock().unwrap();
        assert_eq!(
            actor.source_context(context().scope).err(),
            Some(VoiceError::Capacity)
        );
    }
    clock.outside.store(true, Ordering::Release);
    assert!(actor.source_context(context().scope).is_ok());
    assert!(clock.outside.load(Ordering::Acquire));
}
