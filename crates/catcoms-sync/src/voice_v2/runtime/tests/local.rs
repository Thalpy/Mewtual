use super::*;

#[derive(Debug)]
struct ResponseFinalClock {
    inner: ManualClock,
    revoke: Mutex<Option<CallLease>>,
    advance: AtomicU64,
}
impl Clock for ResponseFinalClock {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        let revoked = self.revoke.lock().unwrap().take();
        if let Some(revoked) = revoked {
            revoked.revoke();
        }
        self.inner
            .advance_ms(self.advance.swap(0, Ordering::AcqRel));
        self.inner.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}

#[test]
fn final_response_ticket_last_clock_cannot_accept_expiry_or_original_owner_close() {
    for fault in 0..4 {
        let clock = Arc::new(ResponseFinalClock {
            inner: ManualClock::new(1000),
            revoke: Mutex::new(None),
            advance: AtomicU64::new(0),
        });
        let runtime = Runtime::new(clock.clone());
        let actor = active(&runtime, 1);
        let ticket = actor
            .admit_transport_verification(device(2), b"exact signed response")
            .unwrap();
        match fault {
            0 => {
                clock.advance.store(6000, Ordering::Release);
            }
            1 => {
                *clock.revoke.lock().unwrap() = Some(actor.gate.live.clone());
            }
            2 => {
                *clock.revoke.lock().unwrap() = Some(
                    actor.group.state.lock().unwrap().members[&device(2)]
                        .live
                        .clone(),
                );
            }
            _ => {
                *clock.revoke.lock().unwrap() =
                    Some(actor.gate.policy.lock().unwrap().live.clone());
            }
        }
        assert_eq!(
            ticket.check_response(&actor, b"exact signed response"),
            Err(VoiceError::Unauthorized),
            "final native owner {fault}"
        );
    }
}

#[test]
fn final_response_ticket_clock_is_outside_guards_and_busy_views_refuse_without_waiting() {
    let clock = Arc::new(OriginClockProbe {
        inner: ManualClock::new(1000),
        owners: Mutex::new(None),
        outside: AtomicBool::new(true),
        samples: AtomicU64::new(0),
    });
    let runtime = Runtime::new(clock.clone());
    let actor = active(&runtime, 1);
    let ticket = actor
        .admit_transport_verification(device(2), b"exact response")
        .unwrap();
    *clock.owners.lock().unwrap() =
        Some((Arc::downgrade(&actor.group), Arc::downgrade(&actor.gate)));
    ticket.check_response(&actor, b"exact response").unwrap();
    assert!(clock.samples.load(Ordering::Acquire) > 0);
    assert!(clock.outside.load(Ordering::Acquire));
    *clock.owners.lock().unwrap() = None;
    assert_eq!(
        actor.with_limits(|_| ticket.check_response(&actor, b"exact response")),
        Err(VoiceError::Capacity)
    );
    assert_eq!(
        actor.with_held_policy_for_test(|| ticket.check_response(&actor, b"exact response")),
        Err(VoiceError::Capacity)
    );
    assert_eq!(
        ticket.check_response(&actor, b"changed response"),
        Err(VoiceError::Unauthorized)
    );
}

#[test]
fn final_response_ticket_cannot_bypass_or_invoke_a_bound_incoming_driver() {
    #[derive(Debug)]
    struct Driver(Arc<AtomicU64>);
    #[async_trait::async_trait]
    impl catcoms_rt::RequestAuthority for Driver {
        fn authorized(&self) -> bool {
            self.0.fetch_add(1, Ordering::AcqRel);
            true
        }
        async fn revoked(&self) {
            futures::future::pending::<()>().await;
        }
    }
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = active(&runtime, 1);
    let calls = Arc::new(AtomicU64::new(0));
    let ticket = actor
        .admit_transport_verification(device(2), b"incoming request")
        .unwrap()
        .bind_request(Arc::new(Driver(calls.clone())))
        .unwrap();
    calls.store(0, Ordering::Release);
    assert_eq!(
        ticket.check_response(&actor, b"incoming request"),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        calls.load(Ordering::Acquire),
        0,
        "response-only check refuses before any incoming callback"
    );
}

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
fn ready_member_prepayment_survives_full_ordinary_capacity_without_refilling_traffic() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let mut actor = active(&runtime, 1);
    actor.sync_view(view(2, &[1, 2, 3, 4])).unwrap();
    let context = crate::voice_v2::Context {
        epoch: 2,
        ..context()
    };
    let mut helpers = [device(3), device(4)];
    helpers.sort();
    let before = runtime.resources().counts();
    actor
        .prepare_ready_members(device(2), &helpers, &context)
        .unwrap();
    assert_eq!(runtime.resources().counts().0 - before.0, 4 * 2048);
    let members = [device(1), device(2), device(3), device(4)];
    actor
        .with_limits(|limits| {
            for member in members {
                for _ in 0..40 {
                    limits.origin(member, crate::voice_v2::limits::Traffic::Setup, 1000)?;
                }
                assert_eq!(
                    limits.origin(member, crate::voice_v2::limits::Traffic::Setup, 1000),
                    Err(VoiceError::RateLimited)
                );
            }
            Ok(())
        })
        .unwrap();
    let pool = runtime.resources();
    let pressure = pool
        .charge(
            context.scope.group,
            crate::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    actor
        .prepare_ready_members(device(2), &helpers, &context)
        .unwrap();
    assert_eq!(pool.counts(), full);
    actor
        .with_limits(|limits| {
            for member in members {
                assert_eq!(
                    limits.origin(member, crate::voice_v2::limits::Traffic::Setup, 1000),
                    Err(VoiceError::RateLimited),
                    "row preparation cannot reset exhausted buckets"
                );
            }
            Ok(())
        })
        .unwrap();
    drop(pressure);
}

#[test]
fn ready_member_prepayment_checks_actual_native_membership_and_policy_before_payment() {
    for kind in [
        "unknown helper",
        "duplicate",
        "endpoint",
        "self",
        "too many",
        "policy",
        "epoch",
        "direct",
        "foreign scope",
    ] {
        let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
        let mut actor = active(&runtime, 1);
        let mut observed = context();
        let mut peer = device(2);
        let mut helpers = vec![device(3)];
        let expected = match kind {
            "duplicate" => {
                helpers.push(device(3));
                VoiceError::Malformed
            }
            "endpoint" => {
                helpers = vec![peer];
                VoiceError::Malformed
            }
            "self" => {
                peer = device(1);
                VoiceError::Unauthorized
            }
            "too many" => {
                helpers = vec![device(3), device(4), device(5)];
                VoiceError::Bounds
            }
            "unknown helper" => {
                helpers = vec![device(4)];
                VoiceError::Unauthorized
            }
            "policy" => {
                observed.scope.policy = Some([9; 32]);
                VoiceError::Unauthorized
            }
            "epoch" => {
                observed.epoch += 1;
                VoiceError::Unauthorized
            }
            "foreign scope" => {
                observed.scope.group = [9; 32];
                VoiceError::Unauthorized
            }
            "direct" => {
                let mut next = view(1, &[1, 2, 3]);
                next.indirect = false;
                actor.sync_view(next).unwrap();
                observed.indirect = false;
                VoiceError::Unauthorized
            }
            _ => unreachable!(),
        };
        let before = runtime.resources().counts();
        assert_eq!(
            actor.prepare_ready_members(peer, &helpers, &observed),
            Err(expected),
            "{kind}"
        );
        assert_eq!(
            runtime.resources().counts(),
            before,
            "{kind} cannot create rows"
        );
    }
}

#[test]
fn ready_member_prepayment_samples_outside_native_guards_and_refuses_contention() {
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
    {
        let _group = actor.group.state.lock().unwrap();
        assert_eq!(
            actor.prepare_ready_members(device(2), &[device(3)], &context()),
            Err(VoiceError::Capacity)
        );
    }
    {
        let _policy = actor.gate.policy.lock().unwrap();
        assert_eq!(
            actor.prepare_ready_members(device(2), &[device(3)], &context()),
            Err(VoiceError::Capacity)
        );
    }
    clock.outside.store(true, Ordering::Release);
    actor
        .prepare_ready_members(device(2), &[device(3)], &context())
        .unwrap();
    assert!(clock.outside.load(Ordering::Acquire));
    runtime.revoke_session();
    assert_eq!(
        actor.prepare_ready_members(device(2), &[device(3)], &context()),
        Err(VoiceError::Unauthorized)
    );
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
