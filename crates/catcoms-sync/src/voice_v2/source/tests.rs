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
        let mut rng = ChaCha20Rng::seed_from_u64(u64::from(message));
        let reserved = ledger
            .reserve(
                signer(1).verifying_key().to_bytes(),
                ReserveRequest {
                    handshake: [9; 32],
                    kind: EncryptedKind::Offer,
                    offer: None,
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
            .err(),
        Some(VoiceError::Reservation),
        "late old direct ACK cannot replace selected helper flight"
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
        SourceOutcome::Unconfirmed
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
fn source_max_payload_retained_b_capacity_refusal_does_not_burn_d_attempt() {
    let f = Fixture::new(true);
    let packet = f.targeted(24, 32 * 1024 + 16);
    // The real native outgoing ledger reservation remains charged alongside SourceLedger.
    f.admit(packet).unwrap();
    let first = f.jobs(&[2, 4]).pop().unwrap();
    let lower = first.keepalive();
    drop(first);
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
