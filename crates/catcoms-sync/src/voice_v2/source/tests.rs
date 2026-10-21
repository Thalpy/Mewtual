use super::super::packets::Authority;
use super::super::presence::{LocalJoin, WatchAuthority, Watcher};
use super::super::runtime::{Runtime, View};
use super::*;
use catcoms_rt::ManualClock;
use rand_chacha::{rand_core::SeedableRng, ChaCha20Rng};
use std::cell::RefCell;
fn signer(n: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[n; 32])
}
fn id(n: u8) -> DeviceId {
    signer(n).device_id()
}
fn key(n: u8) -> JoinKey {
    JoinKey::new([n; 32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap()
}
struct Fixture {
    runtime: Arc<Runtime>,
    clock: Arc<ManualClock>,
    actor: ActorOwner,
    source: Arc<SourceLedger>,
    context: Context,
    join: RefCell<LocalJoin>,
    lease: CallLease,
    room: RoomAuthority,
}

#[test]
fn exact_cached_source_retry_survives_forward_wall_correction_without_new_work() {
    let f = Fixture::new(true);
    let object = f.targeted(22, 32);
    assert_eq!(f.admit(object.clone()), Ok(false));
    let job = f.jobs(&[3]).pop().unwrap();
    let before = f.runtime.resources().counts();
    let counts = f.source.counts();
    f.clock.set_wall_ms(1001);
    assert_eq!(f.admit(object), Ok(true));
    assert_eq!(f.source.counts(), counts);
    assert_eq!(f.runtime.resources().counts(), before);
    assert!(job.authority().authorized());
}

#[test]
fn collection_preserves_actual_source_custody_until_lower_completion_after_lock() {
    let f = Fixture::new(true);
    f.admit(f.targeted(23, 32)).unwrap();
    let job = f.jobs(&[3]).pop().unwrap();
    let lower = job.keepalive();
    f.runtime.revoke_session();
    f.clock.advance_ms(60_001);
    let held = f.runtime.resources().counts().0;
    f.runtime.collect_expired();
    assert_eq!(f.source.counts(), (1, 1, 1));
    // Origin/cursor records can expire, but the active native packet/flight stays charged.
    let after = f.runtime.resources().counts().0;
    assert!(after <= held);
    assert!(!job.authority().authorized());
    assert_eq!(
        f.source.complete(job.finish(None)).unwrap(),
        SourceOutcome::Retired
    );
    f.runtime.collect_expired();
    assert_eq!(f.runtime.resources().counts().0, after);
    assert_eq!(f.source.counts(), (1, 1, 1));
    drop(lower);
    f.runtime.collect_expired();
    assert_eq!(f.source.counts(), (0, 0, 0));
    assert!(f.runtime.resources().counts().0 < after);
}
impl Fixture {
    fn new(indirect: bool) -> Self {
        let clock = Arc::new(ManualClock::new(1000));
        let runtime = Runtime::new(clock.clone());
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        let scope = Scope::for_group(b"native source group", indirect.then_some([8; 32]), 1);
        let (mut actor, registration) = runtime.prepare_actor(scope.group, id(1), &ticket).unwrap();
        registration.register().unwrap();
        actor
            .sync_view(View {
                epoch: 7,
                members: (1..=12).map(id).collect(),
                policy: scope.policy,
                eligible: true,
                indirect,
            })
            .unwrap();
        let context = Context {
            scope,
            local: id(1),
            now: 1000,
            epoch: 7,
            indirect,
        };
        let lease = actor.child_lease().unwrap();
        let source = actor.source().unwrap();
        let join = LocalJoin::new(&context, key(1), lease.clone(), runtime.resources()).unwrap();
        Self {
            runtime,
            clock,
            actor,
            source,
            context,
            join: RefCell::new(join),
            lease,
            room: RoomAuthority {
                scope,
                lease: CallLease::default(),
                custody: None,
            },
        }
    }
    fn activity(&self) -> SourceActivity {
        self.join
            .borrow()
            .source_activity(&Authority {
                context: &self.context,
                lease: &self.lease,
                member: &|_| true,
            })
            .unwrap()
    }
    fn announcement(&self, message: u8, renewal: u64) -> SignedObject {
        let now = self.runtime.clock().now_ms();
        SignedObject::issue(
            self.context.scope,
            &signer(1),
            [message; 32],
            now,
            now + 30_000,
            Body::Announcement {
                key: key(1),
                epoch: 7,
                renewal,
            },
        )
        .unwrap()
    }
    fn targeted(&self, message: u8, bytes: usize) -> SignedObject {
        self.targeted_kind(message, bytes, EncryptedKind::Ice)
    }
    fn targeted_kind(&self, message: u8, bytes: usize, kind: EncryptedKind) -> SignedObject {
        use super::super::packets::{PairBinding, ReserveRequest};
        let mut join = self.join.borrow_mut();
        let ledger = join.packet_ledger_for_test();
        let authority = Authority {
            context: &self.context,
            lease: &self.lease,
            member: &|_| true,
        };
        // Set up the real native ledger owner without claiming WebCrypto confirmation. These
        // tests then use its actual nonce reservation, signed cache and pair retirement paths.
        ledger
            .register_pair(
                PairBinding {
                    peer: id(3),
                    join: key(3).join,
                    key_ref: key(3).key_ref,
                    handshake: [9; 32],
                },
                &authority,
            )
            .unwrap();
        ledger.establish(&[9; 32], &authority).unwrap();
        if kind == EncryptedKind::Offer {
            self.actor
                .with_limits(|limits| {
                    ledger.prepare_sdp_exchange(
                        [9; 32],
                        &authority,
                        limits,
                        &[id(2), id(4)],
                        self.runtime.clock().as_ref(),
                    )
                })
                .unwrap();
        }
        let mut rng = ChaCha20Rng::seed_from_u64(u64::from(message));
        let reserved = ledger
            .reserve(
                signer(1).verifying_key().to_bytes(),
                ReserveRequest {
                    handshake: [9; 32],
                    kind,
                    offer: if kind == EncryptedKind::Offer {
                        None
                    } else {
                        Some(OfferId {
                            issuer: id(1),
                            message: [19; 32],
                        })
                    },
                    ciphertext_bytes: bytes,
                },
                &authority,
                self.runtime.clock().as_ref(),
                &mut rng,
            )
            .unwrap();
        ledger
            .submit(
                reserved.id,
                &vec![5; bytes],
                &signer(1),
                &authority,
                self.runtime.clock().as_ref(),
            )
            .unwrap()
    }
    fn packet_activity(&self, object: &SignedObject) -> Result<SourceActivity, VoiceError> {
        self.join.borrow().source_packet_activity(
            object,
            &Authority {
                context: &self.context,
                lease: &self.lease,
                member: &|_| true,
            },
            self.runtime.clock(),
        )
    }
    fn maximum_answer(&self, offer: &SignedObject) -> SignedObject {
        let Body::Encrypted { header, .. } = offer.body() else {
            unreachable!()
        };
        SignedObject::issue(
            self.context.scope,
            &signer(3),
            [120; 32],
            1000,
            16_000,
            Body::Encrypted {
                header: EncryptedHeader {
                    source_join: key(3).join,
                    target: id(1),
                    target_join: key(1).join,
                    source_key_ref: key(3).key_ref,
                    target_key_ref: key(1).key_ref,
                    handshake: header.handshake,
                    low_to_high: id(3) < id(1),
                    kind: EncryptedKind::Answer,
                    sequence: 1,
                    offer: header.offer,
                },
                ciphertext: vec![6; MAX_SDP_PLAINTEXT + 16],
            },
        )
        .unwrap()
    }
    fn admit(&self, object: SignedObject) -> Result<bool, VoiceError> {
        let activity = if matches!(object.body(), Body::Encrypted { .. }) {
            self.packet_activity(&object)?
        } else {
            self.activity()
        };
        self.source
            .admit(&self.actor, object, activity, self.room.clone())
    }
    fn jobs(&self, seeds: &[u8]) -> Vec<SourceJob> {
        self.source.take_ready(&self.actor, &routes(seeds)).unwrap()
    }
}
fn routes(seeds: &[u8]) -> Vec<BoundRoute> {
    seeds
        .iter()
        .map(|n| BoundRoute {
            device: id(*n),
            peer: PeerId::from_u64(u64::from(*n)),
        })
        .collect()
}

enum FinalRevocation {
    Lease(CallLease),
    Session(Arc<Runtime>),
}
struct FinalPacketClock {
    inner: Arc<ManualClock>,
    target: FinalRevocation,
    armed: std::sync::atomic::AtomicBool,
}
redacted_debug!(FinalPacketClock);
impl Clock for FinalPacketClock {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        if self.armed.swap(false, Ordering::AcqRel) {
            match &self.target {
                FinalRevocation::Lease(owner) => owner.revoke(),
                FinalRevocation::Session(runtime) => runtime.revoke_session(),
            }
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

#[test]
fn queued_source_final_packet_clock_revocation_closes_the_same_lower_observation() {
    for owner in ["call", "room", "session"] {
        let f = Fixture::new(true);
        let object = f.targeted(90, 32);
        let target = match owner {
            "call" => FinalRevocation::Lease(f.lease.clone()),
            "room" => FinalRevocation::Lease(f.room.lease.clone()),
            "session" => FinalRevocation::Session(f.runtime.clone()),
            _ => unreachable!(),
        };
        let clock = Arc::new(FinalPacketClock {
            inner: f.clock.clone(),
            target,
            armed: std::sync::atomic::AtomicBool::new(false),
        });
        let activity = f
            .join
            .borrow()
            .source_packet_activity(
                &object,
                &Authority {
                    context: &f.context,
                    lease: &f.lease,
                    member: &|_| true,
                },
                clock.clone(),
            )
            .unwrap();
        f.source
            .admit(&f.actor, object, activity, f.room.clone())
            .unwrap();
        let job = f.jobs(&[3]).pop().unwrap();
        assert!(job.authority().authorized());
        // The packet's clock is the LAST constituent sampled by this actual lower job. It can
        // revoke earlier hop/origin/room owners too; none may retain a stale positive result.
        clock.armed.store(true, Ordering::Release);
        assert!(!job.authority().authorized(), "{owner}");
    }
}

#[test]
fn queued_source_preserves_the_original_packet_clock_when_monotonic_origins_differ() {
    let f = Fixture::new(true);
    let object = f.targeted(91, 32);
    let packet_clock = Arc::new(ManualClock::new(1000));
    let activity = f
        .join
        .borrow()
        .source_packet_activity(
            &object,
            &Authority {
                context: &f.context,
                lease: &f.lease,
                member: &|_| true,
            },
            packet_clock.clone(),
        )
        .unwrap();
    f.source
        .admit(&f.actor, object, activity, f.room.clone())
        .unwrap();
    let job = f.jobs(&[3]).pop().unwrap();
    assert!(job.authority().authorized());
    packet_clock.advance_ms(MAX_PACKET_LIFETIME_MS);
    packet_clock.set_wall_ms(1000);
    assert_eq!(f.clock.monotonic_ms(), 1000);
    assert_eq!(f.clock.now_ms(), packet_clock.now_ms());
    assert!(
        !job.authority().authorized(),
        "a valid-looking job clock cannot revive the expired original packet clock"
    );
}

#[test]
fn source_later_effect_contention_preserves_prepared_jobs_and_the_unstarted_packet_allowance() {
    let f = Fixture::new(true);
    let a = f.targeted(93, 32);
    let b = f.targeted(94, 32);
    f.admit(a.clone()).unwrap();
    f.admit(b.clone()).unwrap();
    let mut keys = [(a.issuer(), a.id()), (b.issuer(), b.id())];
    keys.sort();
    let mut calls = 0;
    let mut jobs = f
        .source
        .take_ready_using(&f.actor, &routes(&[3]), 4, |origin, target, expires| {
            calls += 1;
            assert_eq!(origin, f.context.local);
            assert_eq!(target, id(3));
            if calls == 2 {
                Err(VoiceError::Capacity)
            } else {
                f.actor.effect(origin, target, expires)
            }
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(
        jobs.len(),
        1,
        "earlier debited job must escape rather than be dropped"
    );
    let first = jobs.pop().unwrap();
    assert_eq!(first.key, keys[0]);
    assert_eq!(first.hop(), SourceHop::Direct(id(3)));
    assert!(first.authority().authorized());
    {
        let state = f.source.state.lock().unwrap();
        let ready = state.packets[&keys[0]].state.lock().unwrap();
        assert_eq!(ready.attempts, 1);
        assert_eq!(ready.direct, BTreeSet::from([id(3)]));
        let unstarted = state.packets[&keys[1]].state.lock().unwrap();
        assert_eq!(unstarted.attempts, 0);
        assert_eq!(unstarted.selected, 0);
        assert!(unstarted.direct.is_empty() && unstarted.helpers.is_empty());
    }
    let mut resumed = f.jobs(&[3]);
    assert_eq!(
        resumed.len(),
        1,
        "only the untouched second packet is ready immediately"
    );
    let second = resumed.pop().unwrap();
    assert_eq!(second.key, keys[1]);
    assert_eq!(second.hop(), SourceHop::Direct(id(3)));
    assert!(second.authority().authorized());
    assert_eq!(f.source.counts(), (2, 2, 2));
    drop((first, second));
}

#[test]
fn source_three_total_submissions_mix_direct_and_two_helpers_without_refill() {
    let f = Fixture::new(true);
    let packet = f.targeted(21, 48);
    f.admit(packet.clone()).unwrap();
    let direct = f.jobs(&[2, 3, 4]).pop().unwrap();
    assert_eq!(direct.hop(), SourceHop::Direct(id(3)));
    f.clock.advance_ms(HEDGE_MS);
    let first = f.jobs(&[2, 3, 4]).pop().unwrap();
    f.clock.advance_ms(HEDGE_MS);
    let second = f.jobs(&[2, 3, 4]).pop().unwrap();
    assert!(matches!(first.hop(), SourceHop::Helper(_)));
    assert!(matches!(second.hop(), SourceHop::Helper(_)));
    assert_ne!(first.hop(), second.hop());
    assert_eq!(f.source.counts(), (1, 3, 3));
    assert!(f.admit(packet).unwrap());
    f.clock.advance_ms(HEDGE_MS);
    assert!(f.jobs(&[2, 3, 4, 5]).is_empty());
    assert_eq!(
        f.source
            .complete(direct.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
            .unwrap(),
        SourceOutcome::EndpointAdmitted,
        "a valid delayed target ACK still confirms the original immutable packet"
    );
    assert_eq!(
        f.source
            .complete(first.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(
        f.source
            .complete(second.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .unwrap(),
        SourceOutcome::EndpointAdmitted
    );
    assert_eq!(f.source.counts(), (1, 3, 0));
}

#[test]
fn source_cached_identity_refuses_changed_content_without_replacing_original_owner() {
    let f = Fixture::new(true);
    let original = f.announcement(20, 1);
    f.admit(original.clone()).unwrap();
    assert_eq!(
        f.admit(f.announcement(20, 2)),
        Err(VoiceError::RetryMismatch)
    );
    assert_eq!(f.source.counts(), (1, 0, 0));
    let job = f.jobs(&[3]).pop().unwrap();
    assert_eq!(job.object_hash(), original.hash());
    assert_eq!(
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
            .unwrap(),
        SourceOutcome::Pending
    );
}
#[test]
fn source_b_to_d_hedge_preserves_each_actual_lower_owner_and_current_selection() {
    let f = Fixture::new(true);
    f.admit(f.targeted(22, 48)).unwrap();
    let first = f.jobs(&[2, 4]).pop().unwrap();
    let lower = first.keepalive();
    assert!(f.jobs(&[2, 4]).is_empty());
    f.clock.advance_ms(HEDGE_MS);
    let second = f.jobs(&[2, 4]).pop().unwrap();
    assert_ne!(first.hop(), second.hop());
    assert_eq!(
        f.source
            .complete(first.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(f.source.counts().2, 2);
    drop(lower);
    assert_eq!(f.source.counts().2, 1);
    assert_eq!(
        f.source
            .complete(second.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .unwrap(),
        SourceOutcome::Pending
    );
    f.clock.advance_ms(HEDGE_MS);
    let direct = f.jobs(&[2, 3, 4]).pop().unwrap();
    assert_eq!(
        direct.hop(),
        SourceHop::Direct(id(3)),
        "new direct final route can use the remaining attempt"
    );
    assert_eq!(
        f.source
            .complete(direct.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
            .unwrap(),
        SourceOutcome::EndpointAdmitted
    );
    f.clock.advance_ms(HEDGE_MS);
    assert!(f.jobs(&[2, 3, 4]).is_empty());
}
#[test]
fn source_room_direct_ack_is_one_recipient_and_fresh_announcements_rotate_coverage() {
    let f = Fixture::new(true);
    let mut direct = BTreeSet::new();
    let mut helpers = BTreeSet::new();
    for renewal in 1..=10 {
        f.admit(f.announcement(30 + renewal as u8, renewal))
            .unwrap();
        let job = f.jobs(&(2..=11).collect::<Vec<_>>()).pop().unwrap();
        assert!(matches!(job.hop(), SourceHop::Direct(_)));
        direct.insert(job.hop().device());
        assert_eq!(
            f.source
                .complete(job.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
                .unwrap(),
            SourceOutcome::Pending
        );
        f.clock.advance_ms(HEDGE_MS);
        let job = f.jobs(&(2..=11).collect::<Vec<_>>()).pop().unwrap();
        helpers.insert(job.hop().device());
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .unwrap();
        f.clock.advance_ms(HEDGE_MS);
        let job = f.jobs(&(2..=11).collect::<Vec<_>>()).pop().unwrap();
        helpers.insert(job.hop().device());
        assert_eq!(
            f.source
                .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
                .unwrap(),
            SourceOutcome::Unconfirmed
        );
        assert_eq!(
            f.source.state.lock().unwrap().packets[&(id(1), [30 + renewal as u8; 32])]
                .state
                .lock()
                .unwrap()
                .attempts,
            3
        );
        f.clock.advance_ms(4000);
    }
    assert_eq!(direct.len(), 10);
    assert!(
        helpers.len() > 2,
        "helper rotation spans fresh original announcements"
    );
}
#[test]
fn source_direct_legacy_never_selects_helper_and_missing_route_spends_no_attempt() {
    let f = Fixture::new(false);
    f.admit(f.targeted(23, 48)).unwrap();
    assert!(f.jobs(&[2, 4]).is_empty());
    assert_eq!(f.source.counts(), (1, 0, 0));
    let job = f.jobs(&[3]).pop().unwrap();
    assert_eq!(
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .err(),
        Some(VoiceError::Unauthorized)
    );
    f.clock.advance_ms(HEDGE_MS);
    let job = f.jobs(&[3]).pop().unwrap();
    assert_eq!(
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
            .unwrap(),
        SourceOutcome::EndpointAdmitted
    );
}
#[test]
fn source_old_first_admission_cannot_use_replacement_join_or_watcher_activity() {
    let f = Fixture::new(true);
    let lease = f.actor.child_lease().unwrap();
    let replacement =
        LocalJoin::new(&f.context, key(2), lease.clone(), f.runtime.resources()).unwrap();
    let replacement = replacement
        .source_activity(&Authority {
            context: &f.context,
            lease: &lease,
            member: &|_| true,
        })
        .unwrap();
    for packet in [f.announcement(61, 1), f.targeted(62, 48)] {
        assert_eq!(
            f.source
                .admit(&f.actor, packet, replacement.clone(), f.room.clone()),
            Err(VoiceError::Unauthorized)
        );
    }
    let watch = f.actor.watch_lease().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(11);
    let mut watcher =
        Watcher::new(&f.context, watch.clone(), f.runtime.resources(), &mut rng).unwrap();
    let authority = WatchAuthority {
        context: &f.context,
        lease: &watch,
        member: &|_| true,
    };
    let activity = watcher.source_activity(&authority).unwrap();
    assert_eq!(
        f.source.admit(
            &f.actor,
            f.announcement(63, 1),
            activity.clone(),
            f.room.clone()
        ),
        Err(VoiceError::Unauthorized)
    );
    let announcement = SignedObject::issue(
        f.context.scope,
        &signer(3),
        [64; 32],
        1000,
        31_000,
        Body::Announcement {
            key: key(3),
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap();
    let challenge = f
        .actor
        .with_limits(|limits| {
            watcher.challenge(
                &announcement,
                &signer(1),
                &authority,
                limits,
                f.runtime.clock().as_ref(),
                &mut rng,
            )
        })
        .unwrap();
    let newer_watch = f.actor.watch_lease().unwrap();
    let newer = Watcher::new(
        &f.context,
        newer_watch.clone(),
        f.runtime.resources(),
        &mut rng,
    )
    .unwrap();
    let newer_activity = newer
        .source_activity(&WatchAuthority {
            context: &f.context,
            lease: &newer_watch,
            member: &|_| true,
        })
        .unwrap();
    assert_eq!(
        f.source
            .admit(&f.actor, challenge.clone(), newer_activity, f.room.clone()),
        Err(VoiceError::Unauthorized)
    );
    assert!(!f
        .source
        .admit(&f.actor, challenge, activity, f.room.clone())
        .unwrap());
    assert_eq!(f.source.counts().0, 1);
}
#[tokio::test]
async fn source_watcher_parent_end_revokes_queued_work_without_an_actor_turn() {
    use futures::FutureExt;
    let f = Fixture::new(true);
    // The same parent seam covers an implicit call watcher and an explicit controller watcher.
    // Neither cancellation waits for the Root to process End or stop-observing.
    for seed in [811, 812] {
        let parent = CallLease::default();
        let watch = f
            .actor
            .watch_lease()
            .unwrap()
            .with_parent(parent.clone())
            .unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let mut watcher =
            Watcher::new(&f.context, watch.clone(), f.runtime.resources(), &mut rng).unwrap();
        assert!(
            watch.clone().with_parent(CallLease::default()).is_err(),
            "parent binding is immutable after state claim"
        );
        let authority = WatchAuthority {
            context: &f.context,
            lease: &watch,
            member: &|_| true,
        };
        let announcement = SignedObject::issue(
            f.context.scope,
            &signer(3),
            [91; 32],
            1000,
            31_000,
            Body::Announcement {
                key: key(3),
                epoch: 7,
                renewal: 1,
            },
        )
        .unwrap();
        let challenge = f
            .actor
            .with_limits(|limits| {
                watcher.challenge(
                    &announcement,
                    &signer(1),
                    &authority,
                    limits,
                    f.runtime.clock().as_ref(),
                    &mut rng,
                )
            })
            .unwrap();
        let activity = watcher.source_activity(&authority).unwrap();
        f.source
            .admit(
                &f.actor,
                challenge.clone(),
                activity.clone(),
                f.room.clone(),
            )
            .unwrap();
        let job = f.jobs(&[2]).pop().unwrap();
        let lower = job.keepalive();
        let gate = job.authority();
        let mut wait = Box::pin(gate.revoked());
        assert!(wait.as_mut().now_or_never().is_none());
        let charged = f.runtime.resources().counts();
        parent.revoke();
        assert!(!gate.authorized());
        assert!(
            wait.as_mut().now_or_never().is_some(),
            "parent wakes driver cancellation immediately"
        );
        assert_eq!(
            f.source
                .admit(&f.actor, challenge, activity, f.room.clone()),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(
            f.source.complete(job.finish(None)).unwrap(),
            SourceOutcome::Retired
        );
        assert_eq!(
            f.runtime.resources().counts(),
            charged,
            "upper cancellation retains actual lower charges"
        );
        drop(lower);
        assert!(f.runtime.resources().counts().0 < charged.0);
    }
}
#[test]
fn maximum_offer_cache_and_successful_lower_custody_allow_maximum_answer_verification() {
    for delayed_lower_retirement in [false, true] {
        let f = Fixture::new(true);
        let offer = f.targeted_kind(
            119,
            super::super::MAX_SDP_PLAINTEXT + 16,
            EncryptedKind::Offer,
        );
        let Body::Encrypted { header: sent, .. } = offer.body() else {
            unreachable!()
        };
        let answer = SignedObject::issue(
            f.context.scope,
            &signer(3),
            [120; 32],
            1000,
            16_000,
            Body::Encrypted {
                header: EncryptedHeader {
                    source_join: key(3).join,
                    target: id(1),
                    target_join: key(1).join,
                    source_key_ref: key(3).key_ref,
                    target_key_ref: key(1).key_ref,
                    handshake: sent.handshake,
                    low_to_high: id(3) < id(1),
                    kind: EncryptedKind::Answer,
                    sequence: 1,
                    offer: sent.offer,
                },
                ciphertext: vec![6; super::super::MAX_SDP_PLAINTEXT + 16],
            },
        )
        .unwrap();
        f.admit(offer.clone()).unwrap();
        let job = f.jobs(&[3]).pop().unwrap();
        let actual_lower = job.keepalive();
        assert_eq!(
            f.source
                .complete(job.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
                .unwrap(),
            SourceOutcome::EndpointAdmitted
        );
        let held = f.runtime.resources().counts();
        assert!(
            f.admit(offer).unwrap(),
            "successful ACK preserves the exact source cache"
        );
        assert_eq!(f.runtime.resources().counts(), held);
        let actual_lower = delayed_lower_retirement.then_some(actual_lower);
        let frame = Envelope::Direct(answer).encode();
        let ticket = f.actor.admit_transport_verification(id(3), &frame).unwrap();
        let actual_verifier = ticket.actual_work();
        let envelope = Envelope::decode(&frame).unwrap();
        ticket.check(&f.actor, &frame).unwrap();
        let incoming = f
            .join
            .borrow_mut()
            .packet_ledger_for_test()
            .admit_inbound(
                envelope.object().clone(),
                &Authority {
                    context: &f.context,
                    lease: &f.lease,
                    member: &|_| true,
                },
                f.runtime.clock().as_ref(),
            )
            .unwrap();
        assert_eq!(
            incoming.ciphertext().len(),
            super::super::MAX_SDP_PLAINTEXT + 16
        );
        assert_eq!(
            f.source.counts(),
            (1, 1, usize::from(delayed_lower_retirement))
        );
        assert!(f.runtime.resources().counts().0 <= 256 * 1024);
        // This is the current packet/verification composition, not an SDP engine/result or
        // crossed-Offer progress claim. Those owners must join a prepaid exchange before V2.
        drop((incoming, ticket, actual_verifier, actual_lower));
    }
}

#[tokio::test]
async fn closing_sdp_exchange_wakes_actual_source_cancellation_without_refunding_lower_custody() {
    let f = Fixture::new(true);
    f.admit(f.targeted_kind(121, MAX_SDP_PLAINTEXT + 16, EncryptedKind::Offer))
        .unwrap();
    let job = f.jobs(&[2]).pop().unwrap();
    let actual_lower = job.keepalive();
    let owner = job.exchange().unwrap();
    let authority = job.authority();
    let mut revoked = Box::pin(authority.revoked());
    assert!(futures::poll!(&mut revoked).is_pending());
    owner.close();
    assert!(
        futures::poll!(&mut revoked).is_ready(),
        "exchange closure wakes the existing lower cancellation waiter"
    );
    assert!(
        f.lease.is_live(),
        "the native call remains distinct from its exchange"
    );
    assert_eq!(
        f.source.complete(job.finish(None)).unwrap(),
        SourceOutcome::Retired
    );
    assert_eq!(
        f.source.counts(),
        (1, 1, 1),
        "retirement refunds neither attempt nor actual lower owner"
    );
    let held = f.runtime.resources().counts().0;
    drop(actual_lower);
    assert_eq!(f.source.counts(), (1, 1, 0));
    assert!(f.runtime.resources().counts().0 < held);
}

#[test]
fn maximum_sdp_reply_and_declared_engine_result_fit_with_two_lowers_under_unrelated_pressure() {
    let f = Fixture::new(true);
    // Readiness must prepay the actual local/peer/two-helper member rows itself. Unrelated
    // pressure applied afterward must not strand a later requester/original transaction.
    let offer = f.targeted_kind(122, MAX_SDP_PLAINTEXT + 16, EncryptedKind::Offer);
    let answer = f.maximum_answer(&offer);
    f.admit(offer).unwrap();
    let first = f.jobs(&[2, 4]).pop().unwrap();
    let owner = first.exchange().unwrap();
    let lower1 = first.keepalive();
    drop(first);
    f.clock.advance_ms(HEDGE_MS);
    let second = f.jobs(&[2, 4]).pop().unwrap();
    let lower2 = second.keepalive();
    drop(second);
    let resources = f.runtime.resources();
    let group = f.context.scope.group;
    let crypto_slots: Vec<_> = (0..3)
        .map(|_| resources.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let forward_slots: Vec<_> = (0..2)
        .map(|_| resources.charge(group, 1, ResourceKind::Forward).unwrap())
        .collect();
    let action_pool = f.join.borrow_mut().packet_ledger_for_test().action_slots();
    let ordinary_actions: Vec<_> = (0..30).map(|_| action_pool.charge(1).unwrap()).collect();
    assert!(action_pool.charge(1).is_err());
    let ordinary = resources
        .charge(
            group,
            256 * 1024 - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let frame = Envelope::Direct(answer).encode();
    let ticket = f.actor.admit_transport_verification(id(3), &frame).unwrap();
    let actual_verifier = ticket.actual_work();
    let envelope = Envelope::decode(&frame).unwrap();
    ticket.check(&f.actor, &frame).unwrap();
    let a = Authority {
        context: &f.context,
        lease: &f.lease,
        member: &|_| true,
    };
    let mut join = f.join.borrow_mut();
    let ledger = join.packet_ledger_for_test();
    let prepared = ledger
        .prepare_inbound(envelope.object().clone(), &a, f.runtime.clock().as_ref())
        .unwrap();
    let event = ledger
        .commit_prepared_inbound(prepared, &a, f.runtime.clock().as_ref(), |admission| {
            f.actor
                .commit_endpoint_origin(&ticket, &envelope, &f.context, |wall, mono| {
                    if !f.room.lease.is_live() {
                        return Err(VoiceError::Unauthorized);
                    }
                    admission.check_at(wall, mono)
                })
        })
        .unwrap();
    drop(join);
    assert!(
        owner
            .decryption_budget()
            .charge(&resources, group, 61_624, ResourceKind::Crypto)
            .is_err(),
        "verification and engine serialize on one actual lane"
    );
    drop((ticket, actual_verifier));
    // General SDP action/IPC dispatch is still disabled. These are its actual native slot,
    // byte and result custody types at their conservative declared maxima, not an AEAD or
    // end-to-end browser claim. The separate real WebCrypto test covers the 8 KiB payload.
    struct RetainedActionInput {
        _input: Vec<u8>,
        _record: super::super::resources::Credit,
        _payload: super::super::resources::Credit,
        _slot: super::super::crypto_actions::slots::Slot,
    }
    let retain = |stage| {
        let (record, payload, issue) = owner
            .prepare_action_credits(stage, 1536, 10_286 - 1536)
            .unwrap();
        let slot = owner.action_slot(&action_pool).unwrap();
        issue.publish();
        RetainedActionInput {
            _input: vec![7; MAX_SDP_PLAINTEXT],
            _record: record,
            _payload: payload,
            _slot: slot,
        }
    };
    let encrypt = retain(super::super::negotiation::Stage::EncryptAction);
    let decrypt = retain(super::super::negotiation::Stage::DecryptAction);
    let engine = owner
        .decryption_budget()
        .charge(&resources, group, 61_624, ResourceKind::Crypto)
        .unwrap();
    let result = owner.retain_result(&vec![b'a'; MAX_SDP_PLAINTEXT]).unwrap();
    assert_eq!(resources.counts(), (256 * 1024, 4));
    f.clock.advance_ms(HEDGE_MS);
    assert!(f.jobs(&[2, 4]).is_empty());
    assert_eq!(
        f.source.counts(),
        (1, 2, 2),
        "a third retained hedge cannot consume reply headroom or an attempt"
    );
    owner.close();
    assert_eq!(
        resources.counts().1,
        4,
        "actual engine retains the retired prepaid lane"
    );
    drop(engine);
    assert_eq!(resources.counts().1, 3);
    let held = resources.counts().0;
    drop((lower1, lower2));
    assert_eq!(f.source.counts(), (1, 2, 0));
    assert!(resources.counts().0 < held);
    drop((
        encrypt,
        decrypt,
        event,
        result,
        ordinary,
        crypto_slots,
        forward_slots,
        ordinary_actions,
    ));
}

#[test]
fn untrusted_peek_lends_only_existing_exact_binding_without_authenticating_a_signature() {
    let f = Fixture::new(true);
    let offer = f.targeted_kind(123, MAX_SDP_PLAINTEXT + 16, EncryptedKind::Offer);
    let answer = f.maximum_answer(&offer);
    let owner = f
        .runtime
        .resources()
        .exchange(f.context.scope.group)
        .unwrap();
    let frame = Envelope::Direct(answer.clone()).encode();
    assert!(owner.lends_verification(id(3), &frame));
    let mut bad_signature = frame.clone();
    *bad_signature.last_mut().unwrap() ^= 1;
    assert!(
        owner.lends_verification(id(3), &bad_signature),
        "a peek selects transient capacity only"
    );
    assert_eq!(
        Envelope::decode(&bad_signature).err(),
        Some(VoiceError::Signature)
    );
    let held = f.runtime.resources().counts();
    let ticket = f
        .actor
        .admit_transport_verification(id(3), &bad_signature)
        .unwrap();
    assert!(Envelope::decode(&bad_signature).is_err());
    drop(ticket);
    assert_eq!(
        f.runtime.resources().counts(),
        held,
        "failed signatures retain no packet/origin admission or extra slot"
    );
    let Body::Encrypted {
        mut header,
        ciphertext,
    } = answer.body().clone()
    else {
        unreachable!()
    };
    header.target_join = [99; 32];
    let other_join = SignedObject::issue(
        f.context.scope,
        &signer(3),
        [121; 32],
        1000,
        16_000,
        Body::Encrypted { header, ciphertext },
    )
    .unwrap();
    assert!(!owner.lends_verification(id(3), &Envelope::Direct(other_join).encode()));
    let permit = RoutePermit::issue(&signer(3), &answer, id(2), 1000, 16_000).unwrap();
    let relay = Envelope::HelperDelivery {
        object: answer,
        permit,
    }
    .encode();
    assert!(owner.lends_verification(id(2), &relay));
    assert!(
        !owner.lends_verification(id(5), &relay),
        "a new authenticated member cannot borrow this exchange's helper headroom"
    );
    assert_eq!(f.source.counts(), (0, 0, 0));
}

#[test]
fn source_max_ice_payload_retained_b_capacity_refusal_does_not_burn_d_attempt() {
    let f = Fixture::new(true);
    let packet = f.targeted(24, EncryptedKind::Ice.max_plaintext() + 16);
    // The real native outgoing ledger reservation remains charged alongside SourceLedger.
    f.admit(packet).unwrap();
    let first = f.jobs(&[2, 4]).pop().unwrap();
    let lower = first.keepalive();
    drop(first);
    // Capacity pressure, rather than an oversized supported SDP, pins the invariant:
    // a refused hedge must not burn an attempt, and actual lower retirement makes room.
    let current = f.runtime.resources().counts().0;
    let fill = f
        .runtime
        .resources()
        .charge(
            f.context.scope.group,
            256 * 1024 - current - 10,
            ResourceKind::Retained,
        )
        .unwrap();
    f.clock.advance_ms(HEDGE_MS);
    for _ in 0..10 {
        assert!(f.jobs(&[2, 4]).is_empty());
    }
    assert_eq!(f.source.counts(), (1, 1, 1));
    drop(lower);
    let second = f.jobs(&[2, 4]).pop().unwrap();
    assert_eq!(f.source.counts().1, 2);
    assert!(matches!(second.hop(), SourceHop::Helper(_)));
    drop(second);
    drop(fill);
}
#[test]
fn source_refused_capacity_spends_no_original_quota_and_room_revocation_stops_actual_work() {
    let f = Fixture::new(true);
    let current = f.runtime.resources().counts().0;
    let fill = f
        .runtime
        .resources()
        .charge(
            f.context.scope.group,
            256 * 1024 - current - 10,
            ResourceKind::Retained,
        )
        .unwrap();
    for _ in 0..10 {
        assert_eq!(f.admit(f.announcement(70, 1)), Err(VoiceError::Capacity));
    }
    assert_eq!(f.source.counts(), (0, 0, 0));
    drop(fill);
    for renewal in 1..=8 {
        f.admit(f.announcement(70 + renewal as u8, renewal))
            .unwrap();
    }
    assert_eq!(f.admit(f.announcement(90, 9)), Err(VoiceError::RateLimited));
    let job = f.jobs(&[2, 3, 4]).pop().unwrap();
    let lower = job.keepalive();
    let before = f.runtime.resources().counts();
    f.room.lease.revoke();
    assert!(!job.authority().authorized());
    assert_eq!(
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::EndpointAdmitted)))
            .unwrap(),
        SourceOutcome::Retired
    );
    assert_eq!(f.runtime.resources().counts(), before);
    drop(lower);
    assert!(f.runtime.resources().counts().0 < before.0);
}
#[test]
fn source_actor_replacement_keeps_protected_packet_and_attempt_count() {
    let f = Fixture::new(true);
    f.admit(f.targeted(25, 48)).unwrap();
    let job = f.jobs(&[2, 4]).pop().unwrap();
    let lower = job.keepalive();
    drop(job);
    let ticket = f.runtime.session_ticket();
    let (mut next, installation) = f
        .runtime
        .prepare_actor(f.context.scope.group, id(1), &ticket)
        .unwrap();
    installation.register().unwrap();
    next.sync_view(View {
        epoch: 7,
        members: (1..=12).map(id).collect(),
        policy: f.context.scope.policy,
        eligible: true,
        indirect: true,
    })
    .unwrap();
    assert!(Arc::ptr_eq(&next.source().unwrap(), &f.source));
    assert_eq!(f.source.counts(), (1, 1, 1));
    assert!(f
        .source
        .take_ready(&next, &routes(&[2, 4]))
        .unwrap()
        .is_empty());
    drop(lower);
    assert_eq!(f.source.counts(), (1, 1, 0));
}

#[test]
fn source_actual_outgoing_pair_retirement_revokes_jobs_and_unseen_packet_admission() {
    let f = Fixture::new(true);
    let queued = f.targeted(97, 48);
    let unseen = f.targeted(98, 48);
    let unseen_activity = f.packet_activity(&unseen).unwrap();
    f.admit(queued).unwrap();
    let job = f.jobs(&[2, 4]).pop().unwrap();
    let lower = job.keepalive();
    let before = f.runtime.resources().counts();
    f.join
        .borrow_mut()
        .packet_ledger_for_test()
        .retire_handshake(&[9; 32]);
    assert!(
        f.lease.is_live(),
        "pair retirement leaves the local join active"
    );
    assert!(!job.authority().authorized());
    assert!(f.jobs(&[2, 4]).is_empty());
    assert_eq!(
        f.source
            .admit(&f.actor, unseen.clone(), unseen_activity, f.room.clone()),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.packet_activity(&unseen).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.source.counts(), (1, 1, 1));
    assert_eq!(
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .unwrap(),
        SourceOutcome::Retired
    );
    assert_eq!(
        f.runtime.resources().counts(),
        before,
        "retirement does not refund an actual lower holder"
    );
    drop(lower);
    assert!(f.runtime.resources().counts().0 < before.0);
}

#[test]
fn source_exact_native_action_retirement_keeps_pair_join_and_lower_credit_distinct() {
    let f = Fixture::new(true);
    let packet = f.targeted(96, 48);
    f.admit(packet.clone()).unwrap();
    let job = f.jobs(&[2, 4]).pop().unwrap();
    let lower = job.keepalive();
    let before = f.runtime.resources().counts();
    let authority = Authority {
        context: &f.context,
        lease: &f.lease,
        member: &|_| true,
    };
    f.join
        .borrow()
        .retire_outgoing(packet.id(), &authority)
        .unwrap();
    assert!(f.lease.is_live() && !job.authority().authorized());
    assert_eq!(
        f.packet_activity(&packet).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(f.jobs(&[2, 4]).is_empty());
    assert_eq!(
        f.source
            .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .unwrap(),
        SourceOutcome::Retired
    );
    assert_eq!(f.runtime.resources().counts(), before);
    drop(lower);
    // An unrelated new outgoing action can still use the current pair and monotonically burned
    // allocator. Retiring one action grants neither a new attempt nor a new packet for old bytes.
    let next = f.targeted(100, 48);
    assert_ne!(next.id(), packet.id());
    let (Body::Encrypted { header: old, .. }, Body::Encrypted { header: new, .. }) =
        (packet.body(), next.body())
    else {
        unreachable!()
    };
    assert!(new.sequence > old.sequence);
    f.admit(next).unwrap();
    assert_eq!(f.jobs(&[2, 4]).len(), 1);
}

#[test]
fn source_expiry_rollback_and_skew_retain_protection_until_actual_lower_drop() {
    let f = Fixture::new(true);
    f.admit(f.announcement(99, 1)).unwrap();
    let job = f.jobs(&[2, 4]).pop().unwrap();
    let lower = job.keepalive();
    drop(job);
    f.clock.advance_ms(30_000);
    assert!(
        f.jobs(&[2, 4]).is_empty(),
        "source submission stops at original expiry"
    );
    assert_eq!(f.source.counts(), (1, 1, 1));
    f.clock.set_wall_ms(1000);
    f.clock.advance_ms(CLOCK_SKEW_MS + 1);
    assert!(f.jobs(&[2, 4]).is_empty());
    assert_eq!(
        f.source.counts(),
        (1, 1, 1),
        "effective time rejects rollback but actual lower owner still protects packet"
    );
    drop(lower);
    assert_eq!(f.source.counts(), (0, 0, 0));
}
