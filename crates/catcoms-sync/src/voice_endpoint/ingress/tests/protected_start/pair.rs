//! Actual Root discovery, immutable pair retry identity and first native admission.
use super::*;
use voice_v2::presence::PairSelector;
use voice_v2::resources::ResourceKind;

#[derive(Default)]
struct Probe {
    reads: usize,
    trigger: usize,
    hook: Option<Box<dyn FnOnce() + Send>>,
}
struct PairClock {
    inner: Arc<ManualClock>,
    probe: Arc<std::sync::Mutex<Probe>>,
}
impl std::fmt::Debug for PairClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairClock { .. }")
    }
}
impl Clock for PairClock {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        let hook = {
            let mut probe = self.probe.lock().unwrap();
            probe.reads += 1;
            if probe.reads == probe.trigger {
                probe.hook.take()
            } else {
                None
            }
        };
        // The hook is not under its control mutex and must also be outside Group/Policy.
        // Actual Runtime session revocation below would deadlock if a production sample moved
        // beneath those guards. This is an original Clock seam, not a fabricated pair gate.
        if let Some(hook) = hook {
            hook();
        }
        self.inner.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.inner.sleep(duration)
    }
}
async fn probed() -> (DiscoveredFixture, Arc<std::sync::Mutex<Probe>>) {
    let probe = Arc::new(std::sync::Mutex::new(Probe::default()));
    let shared = probe.clone();
    let (fixture, _) = low_discovery_with_clock(1, false, move |inner| {
        Arc::new(PairClock {
            inner,
            probe: shared,
        })
    })
    .await;
    *probe.lock().unwrap() = Probe::default();
    (fixture, probe)
}

#[tokio::test]
async fn root_handoff_samples_its_original_clock_before_engine_and_observes_callback_revocation() {
    let ((mut root, remote, runtime, _clock, grant), probe) = probed().await;
    let receipt = root
        .prepare_voice_pair(&grant, selector(&root, &remote))
        .unwrap();
    let action = root
        .prepare_voice_derivation(&grant, receipt.handshake)
        .unwrap();
    let before = runtime.resources().counts();
    let observed_action = action.clone();
    let lease = grant.lease();
    *probe.lock().unwrap() = Probe {
        reads: 0,
        trigger: 1,
        hook: Some(Box::new(move || {
            assert!(
                observed_action.result_fence_available_for_test(),
                "callback-bearing Clock must precede Engine"
            );
            lease.revoke();
        })),
    };
    assert_eq!(
        action.borrow_handoff(16 * 1024).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(
        probe.lock().unwrap().hook.is_none(),
        "actual original Clock ran"
    );
    assert_eq!(
        runtime.resources().counts(),
        before,
        "refusal cannot consume a loan"
    );
}

fn selector(root: &Node, remote: &Node) -> PairSelector {
    PairSelector {
        peer: remote.device_id(),
        join: key(82).join,
        key_ref: key(82).key_ref,
        epoch: root.group.epoch(),
    }
}
fn state(root: &Node) -> (usize, usize, u64) {
    root.voice_runtime
        .as_ref()
        .and_then(|voice| voice.endpoints.participant.as_ref())
        .map_or((0, 0, 0), |participant| {
            participant.join.confirmation_counts_for_test()
        })
}

fn prove_replacement(
    root: &mut Node,
    remote: &mut Node,
    clock: &ManualClock,
    seed: u8,
) -> PairSelector {
    let context = root.voice_v2_context(1).unwrap();
    let remote_context = Context {
        local: remote.device_id(),
        ..context
    };
    let lease = CallLease::default();
    let mut join = LocalJoin::new(
        &remote_context,
        key(seed),
        lease.clone(),
        Arc::new(voice_v2::resources::Resources::default()),
    )
    .unwrap();
    let remote_authority = authority(&remote_context, &lease);
    let announcement = join
        .announce(&remote.device, &remote_authority, clock, &mut remote.rng)
        .unwrap();
    let voice = root.voice_runtime.as_mut().unwrap();
    let participant = voice.endpoints.participant.as_ref().unwrap();
    let observer = voice
        .endpoints
        .observers
        .get_mut(&participant.watch.id)
        .unwrap();
    let watch = WatchAuthority {
        context: &context,
        lease: &observer.lease,
        member: &|_| true,
    };
    let challenge = voice
        .current
        .as_ref()
        .unwrap()
        .with_limits(|limits| {
            observer.watcher.challenge(
                &announcement,
                &root.device,
                &watch,
                limits,
                clock,
                &mut root.rng,
            )
        })
        .unwrap();
    let proof = join
        .answer(
            &challenge,
            &remote.device,
            &remote_authority,
            clock,
            &mut remote.rng,
        )
        .unwrap();
    observer.watcher.prove(&proof, &watch, clock).unwrap();
    PairSelector {
        peer: remote.device_id(),
        join: key(seed).join,
        key_ref: key(seed).key_ref,
        epoch: context.epoch,
    }
}

#[tokio::test]
async fn root_pair_recovers_original_receipt_under_full_pressure_without_resetting_deadline() {
    let (mut root, remote, runtime, clock, grant) = low_discovery_fixture(1).await;
    let request = selector(&root, &remote);
    // Pair admission may follow discovery later. Its independent window must survive the
    // original challenge's expiry, without extending either deadline on exact retry.
    clock.advance_ms(5000);
    let mut changes = root.voice_endpoint_changes().unwrap();
    let first = root.prepare_voice_pair(&grant, request).unwrap();
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    assert_eq!(state(&root), (1, 1, 1));
    let used = runtime.resources().counts().0;
    let held = runtime
        .resources()
        .charge(
            grant.scope().group,
            crate::voice_v2::resources::GROUP_BYTES - used,
            ResourceKind::Retained,
        )
        .unwrap();
    for _ in 0..12 {
        assert_eq!(root.prepare_voice_pair(&grant, request).unwrap(), first);
        assert_eq!(state(&root), (1, 1, 1));
        assert_eq!(
            runtime.resources().counts().0,
            crate::voice_v2::resources::GROUP_BYTES
        );
        assert!(
            !changes.has_changed().unwrap(),
            "exact recovery cannot self-wake its driver"
        );
    }
    drop(held);
    clock.advance_ms(10_000);
    let implicit = root
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .implicit_watch_for_test()
        .unwrap();
    assert!(root.voice_watch_peers(&implicit).unwrap().is_empty());
    assert_eq!(root.prepare_voice_pair(&grant, request).unwrap(), first);
    clock.advance_ms(5000);
    assert!(
        root.prepare_voice_pair(&grant, request).is_err(),
        "exact retry cannot extend the original pair window"
    );
    assert_eq!(state(&root), (1, 1, 1));
}

#[tokio::test]
async fn root_pair_refuses_explicit_watch_evidence_and_foreign_or_changed_selectors() {
    let ((mut root, remote, _runtime, _clock, grant), watch) =
        low_discovery_for_watch(1, true).await;
    let watch = watch.unwrap();
    assert_eq!(
        root.voice_watch_peers(&watch).unwrap().len(),
        1,
        "other watch has actual signed proof"
    );
    let implicit = root
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .implicit_watch_for_test()
        .unwrap();
    assert!(root.voice_watch_peers(&implicit).unwrap().is_empty());
    let request = selector(&root, &remote);
    assert_eq!(
        root.prepare_voice_pair(&grant, request).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(state(&root), (0, 0, 0));

    let (mut root, remote, _runtime, _clock, grant) = low_discovery_fixture(1).await;
    let request = selector(&root, &remote);
    let foreign = Arc::new(CallGrant::from_lease(CallLease::default(), grant.scope()));
    assert_eq!(
        root.prepare_voice_pair(&foreign, request).err(),
        Some(VoiceError::Unauthorized)
    );
    for changed in [
        PairSelector {
            join: [99; 32],
            ..request
        },
        PairSelector {
            key_ref: [99; 32],
            ..request
        },
        PairSelector {
            epoch: request.epoch + 1,
            ..request
        },
    ] {
        assert_eq!(
            root.prepare_voice_pair(&grant, changed).err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(state(&root), (0, 0, 0));
    }
    assert!(root.prepare_voice_pair(&grant, request).is_ok());
    assert_eq!(state(&root), (1, 1, 1));
}

#[tokio::test]
async fn root_pair_local_capacity_refusal_publishes_neither_table_nor_serial() {
    let (mut root, remote, runtime, _clock, grant) = low_discovery_fixture(1).await;
    let request = selector(&root, &remote);
    let used = runtime.resources().counts().0;
    let held = runtime
        .resources()
        .charge(
            grant.scope().group,
            crate::voice_v2::resources::GROUP_BYTES - used,
            ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        root.prepare_voice_pair(&grant, request).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(state(&root), (0, 0, 0));
    assert_eq!(
        runtime.resources().counts().0,
        crate::voice_v2::resources::GROUP_BYTES
    );
    drop(held);
    assert!(root.prepare_voice_pair(&grant, request).is_ok());
    assert_eq!(state(&root), (1, 1, 1));
}

#[tokio::test]
async fn root_pair_creation_epoch_cannot_move_before_first_protected_derivation() {
    let (mut root, remote, _runtime, _clock, grant) = low_discovery_fixture(1).await;
    let request = selector(&root, &remote);
    let receipt = root.prepare_voice_pair(&grant, request).unwrap();
    let third = catcoms_mls::MlsDevice::generate().unwrap();
    root.with_observed_mls_transition(|node| {
        node.group
            .add_member(&node.device, third.key_package().unwrap())
    })
    .unwrap();
    assert!(root.group.contains_device(&remote.device_id()) && grant.lease().is_live());
    assert!(root.group.epoch() > request.epoch);
    assert_eq!(
        root.prepare_voice_derivation(&grant, receipt.handshake)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        root.prepare_voice_pair(&grant, request).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        root.prepare_voice_pair(
            &grant,
            PairSelector {
                epoch: root.group.epoch(),
                ..request
            }
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(state(&root), (1, 1, 1));
}

#[tokio::test]
async fn root_pair_newly_proved_rejoin_cannot_substitute_an_uncertain_original_request() {
    let (mut root, mut remote, runtime, clock, grant) = low_discovery_fixture(1).await;
    let original = selector(&root, &remote);
    let first = root.prepare_voice_pair(&grant, original).unwrap();
    let derivation = root
        .prepare_voice_derivation(&grant, first.handshake)
        .unwrap();
    let replacement = prove_replacement(&mut root, &mut remote, clock.as_ref(), 83);
    assert_eq!(original.peer, replacement.peer);
    assert_ne!(original.join, replacement.join);
    let implicit = root
        .voice_runtime
        .as_ref()
        .unwrap()
        .endpoints
        .implicit_watch_for_test()
        .unwrap();
    let peers = root.voice_watch_peers(&implicit).unwrap();
    assert!(
        matches!(peers[0].announcement().body(), voice_v2::Body::Announcement { key, .. } if key.join == replacement.join)
    );
    // A failed new candidate must leave the earlier independently pinned pair usable. The
    // lost first reply still names its immutable selector, even though discovery now differs.
    let used = runtime.resources().counts().0;
    let held = runtime
        .resources()
        .charge(
            grant.scope().group,
            crate::voice_v2::resources::GROUP_BYTES - used,
            ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        root.prepare_voice_pair(&grant, replacement).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(root.prepare_voice_pair(&grant, original).unwrap(), first);
    assert!(Arc::ptr_eq(
        &derivation,
        &root
            .prepare_voice_derivation(&grant, first.handshake)
            .unwrap()
    ));
    assert!(derivation.is_pending());
    assert_eq!(state(&root), (1, 1, 1));
    drop(held);
    let second = root.prepare_voice_pair(&grant, replacement).unwrap();
    assert_ne!(first.handshake, second.handshake);
    assert_eq!(root.prepare_voice_pair(&grant, original).unwrap(), first);
    assert_eq!(state(&root), (2, 2, 2));
}

#[tokio::test]
async fn root_pair_every_original_clock_boundary_fences_native_owner_revocation() {
    // Observe the actual successful path rather than hardcoding an implementation sample
    // count. Sweep every callback, including its final original native monotonic observation.
    let ((mut root, remote, _runtime, _clock, grant), probe) = probed().await;
    let request = selector(&root, &remote);
    root.prepare_voice_pair(&grant, request).unwrap();
    let samples = probe.lock().unwrap().reads;
    assert!(samples > 1 && samples < 64, "bounded native preparation");
    for owner in ["call", "room", "session"] {
        for at in 1..=samples {
            let ((mut root, remote, runtime, _clock, grant), probe) = probed().await;
            let request = selector(&root, &remote);
            let hook: Box<dyn FnOnce() + Send> = match owner {
                "call" => {
                    let lease = grant.lease();
                    Box::new(move || lease.revoke())
                }
                "room" => {
                    let lease = root.voice_room_authority(1).unwrap().lease;
                    Box::new(move || lease.revoke())
                }
                _ => {
                    let runtime = Arc::downgrade(&runtime);
                    Box::new(move || runtime.upgrade().unwrap().revoke_session())
                }
            };
            *probe.lock().unwrap() = Probe {
                reads: 0,
                trigger: at,
                hook: Some(hook),
            };
            assert!(
                root.prepare_voice_pair(&grant, request).is_err(),
                "{owner} at callback {at}/{samples}"
            );
            assert!(
                probe.lock().unwrap().hook.is_none(),
                "actual callback was reached"
            );
            assert_eq!(
                state(&root),
                (0, 0, 0),
                "neither pair table nor serial may publish"
            );
        }
    }
}

#[tokio::test]
async fn root_pair_final_original_clock_expiry_cannot_publish_a_late_preparation() {
    let ((mut root, remote, _runtime, _clock, grant), probe) = probed().await;
    let request = selector(&root, &remote);
    root.prepare_voice_pair(&grant, request).unwrap();
    let samples = probe.lock().unwrap().reads;
    let ((mut root, remote, _runtime, clock, grant), probe) = probed().await;
    let request = selector(&root, &remote);
    *probe.lock().unwrap() = Probe {
        reads: 0,
        trigger: samples,
        hook: Some(Box::new(move || {
            clock.advance_ms(15_001);
        })),
    };
    assert_eq!(
        root.prepare_voice_pair(&grant, request).err(),
        Some(VoiceError::Freshness)
    );
    assert!(probe.lock().unwrap().hook.is_none());
    assert_eq!(state(&root), (0, 0, 0));
}

#[tokio::test]
async fn root_pair_cached_recovery_rechecks_every_original_clock_owner_boundary() {
    let ((mut root, remote, _runtime, _clock, grant), probe) = probed().await;
    let request = selector(&root, &remote);
    root.prepare_voice_pair(&grant, request).unwrap();
    *probe.lock().unwrap() = Probe::default();
    root.prepare_voice_pair(&grant, request).unwrap();
    let samples = probe.lock().unwrap().reads;
    assert!(samples > 1 && samples < 64);
    for owner in ["call", "room", "session"] {
        for at in 1..=samples {
            let ((mut root, remote, runtime, _clock, grant), probe) = probed().await;
            let request = selector(&root, &remote);
            root.prepare_voice_pair(&grant, request).unwrap();
            let hook: Box<dyn FnOnce() + Send> = match owner {
                "call" => {
                    let lease = grant.lease();
                    Box::new(move || lease.revoke())
                }
                "room" => {
                    let lease = root.voice_room_authority(1).unwrap().lease;
                    Box::new(move || lease.revoke())
                }
                _ => {
                    let runtime = Arc::downgrade(&runtime);
                    Box::new(move || runtime.upgrade().unwrap().revoke_session())
                }
            };
            *probe.lock().unwrap() = Probe {
                reads: 0,
                trigger: at,
                hook: Some(hook),
            };
            assert!(
                root.prepare_voice_pair(&grant, request).is_err(),
                "cached {owner} at {at}/{samples}"
            );
            assert!(probe.lock().unwrap().hook.is_none());
            assert!(
                matches!(state(&root), (0, 0, 0) | (1, 1, 1)),
                "retirement or original pair only; no new allocator"
            );
        }
    }
}

#[tokio::test]
async fn root_pair_final_native_contention_revokes_unpublished_pin_and_preserves_exact_retry() {
    let ((mut root, remote, _runtime, _clock, grant), probe) = probed().await;
    let request = selector(&root, &remote);
    root.prepare_voice_pair(&grant, request).unwrap();
    let samples = probe.lock().unwrap().reads;
    for policy in [false, true] {
        let ((mut root, remote, runtime, _clock, grant), probe) = probed().await;
        let request = selector(&root, &remote);
        let actor = root
            .voice_runtime
            .as_ref()
            .unwrap()
            .current
            .as_ref()
            .unwrap();
        let pins = actor.pair_pin_observer_for_test(request.peer, request.join, request.key_ref);
        let (start, started) = std::sync::mpsc::sync_channel(0);
        let (held, ready) = std::sync::mpsc::sync_channel(0);
        let (release, released) = std::sync::mpsc::channel();
        let hold = actor.native_view_holder_for_test(policy, held, released);
        let holder = std::thread::spawn(move || {
            if started.recv().is_ok() {
                hold();
            }
        });
        let check = pins.clone();
        *probe.lock().unwrap() = Probe {
            reads: 0,
            trigger: samples,
            hook: Some(Box::new(move || {
                assert_eq!(
                    check(),
                    1,
                    "the actual provisional pin exists BEFORE contention"
                );
                start.send(()).unwrap();
                ready.recv().unwrap();
            })),
        };
        let before = runtime.resources().counts();
        let result = root.prepare_voice_pair(&grant, request);
        let fired = probe.lock().unwrap().hook.take().is_none();
        let _ = release.send(());
        holder.join().unwrap();
        assert!(fired);
        assert_eq!(result.err(), Some(VoiceError::Capacity));
        assert_eq!(state(&root), (0, 0, 0));
        assert_eq!(
            pins(),
            0,
            "an unpublished refusal leaves no live origin pin"
        );
        assert_eq!(runtime.resources().counts(), before);
        let first = root.prepare_voice_pair(&grant, request).unwrap();
        assert_eq!(state(&root), (1, 1, 1));
        assert_eq!(pins(), 1);
        assert_eq!(root.prepare_voice_pair(&grant, request).unwrap(), first);
        assert_eq!(pins(), 1);
        // Pair preparation is not traffic: local refusal and admission consume neither the
        // remote member's room burst nor its next supported source operation.
        root.voice_runtime
            .as_ref()
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .with_limits(|limits| {
                for _ in 0..8 {
                    limits.origin(request.peer, voice_v2::limits::Traffic::Room, 0)?;
                }
                assert_eq!(
                    limits.origin(request.peer, voice_v2::limits::Traffic::Room, 0),
                    Err(VoiceError::RateLimited)
                );
                Ok(())
            })
            .unwrap();
    }
}
