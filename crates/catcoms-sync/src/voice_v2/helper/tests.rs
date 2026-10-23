use super::super::runtime::{Runtime, View};
use super::*;
use catcoms_rt::{ManualClock, RequestCancellation};
use tokio::sync::watch;

fn signer(seed: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[seed; 32])
}
fn id(seed: u8) -> DeviceId {
    signer(seed).device_id()
}
fn scope() -> Scope {
    Scope::for_group(b"helper native group", Some([8; 32]), 123)
}
fn view(epoch: u64, seeds: &[u8]) -> View {
    View {
        epoch,
        members: seeds.iter().map(|seed| id(*seed)).collect(),
        policy: scope().policy,
        eligible: true,
        indirect: true,
    }
}
fn key() -> JoinKey {
    let bytes = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    JoinKey::new([1; 32], bytes.try_into().unwrap()).unwrap()
}

#[test]
fn collection_preserves_actual_helper_custody_until_lower_completion_after_lock() {
    let f = Fixture::new();
    f.admit(f.targeted(90)).unwrap();
    let job = f.jobs().pop().unwrap();
    let lower = job.keepalive();
    f.runtime.revoke_session();
    f.clock.advance_ms(60_001);
    f.runtime.collect_expired();
    assert_eq!(f.helper.counts().0, 1);
    let after = f.runtime.resources().counts();
    assert!(!job.authority().authorized());
    f.helper.complete(job.finish(false)).unwrap();
    f.runtime.collect_expired();
    assert_eq!(f.runtime.resources().counts(), after);
    assert_eq!(f.helper.counts().0, 1);
    drop(lower);
    f.runtime.collect_expired();
    assert_eq!(f.helper.counts(), (0, 0, 0));
    assert!(f.runtime.resources().counts().0 < after.0);
}
struct Fixture {
    runtime: Arc<Runtime>,
    clock: Arc<ManualClock>,
    actor: ActorOwner,
    helper: Arc<HelperLedger>,
    context: Context,
    room: RoomAuthority,
    routes: Vec<BoundRoute>,
}
impl Fixture {
    fn new() -> Self {
        let clock = Arc::new(ManualClock::new(1000));
        let runtime = Runtime::new(clock.clone());
        Self::on(runtime, clock, scope())
    }
    fn on(runtime: Arc<Runtime>, clock: Arc<ManualClock>, group_scope: Scope) -> Self {
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        let (mut actor, registration) = runtime
            .prepare_actor(group_scope.group, id(2), &ticket)
            .unwrap();
        registration.register().unwrap();
        actor
            .sync_view(view(7, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]))
            .unwrap();
        let helper = actor.helper().unwrap();
        Self {
            runtime,
            clock,
            actor,
            helper,
            context: Context {
                scope: group_scope,
                local: id(2),
                now: 1000,
                epoch: 7,
                indirect: true,
            },
            room: RoomAuthority {
                scope: group_scope,
                lease: CallLease::default(),
                custody: None,
            },
            routes: (1..=10)
                .map(|seed| BoundRoute {
                    device: id(seed),
                    peer: PeerId::from_u64(u64::from(seed)),
                })
                .collect(),
        }
    }
    fn announcement(&self, message: u8, renewal: u64) -> Envelope {
        let now = self.runtime.clock().now_ms();
        let object = SignedObject::issue(
            self.context.scope,
            &signer(1),
            [message; 32],
            now,
            now + 30_000,
            Body::Announcement {
                key: key(),
                epoch: self.context.epoch,
                renewal,
            },
        )
        .unwrap();
        let permit = RoutePermit::issue(&signer(1), &object, id(2), now, now + 15_000).unwrap();
        Envelope::ToHelper { object, permit }
    }
    fn targeted(&self, message: u8) -> Envelope {
        let now = self.runtime.clock().now_ms();
        let object = SignedObject::issue(
            self.context.scope,
            &signer(1),
            [message; 32],
            now,
            now + 15_000,
            Body::WatchChallenge {
                watcher: [6; 32],
                target: id(3),
                join: [3; 32],
                key_ref: [4; 32],
                challenge: [5; 32],
            },
        )
        .unwrap();
        let permit = RoutePermit::issue(&signer(1), &object, id(2), now, now + 15_000).unwrap();
        Envelope::ToHelper { object, permit }
    }
    fn admit(&self, envelope: Envelope) -> Result<Admission, VoiceError> {
        self.helper.admit(
            &self.actor,
            id(1),
            envelope,
            &self.context,
            &self.room,
            &self.routes,
        )
    }
    fn receipt(&self, envelope: Envelope) -> Result<HelperReceipt, VoiceError> {
        let ticket = self
            .actor
            .admit_transport_verification(id(1), &envelope.encode())?;
        self.helper.admit_verified(
            &self.actor,
            ticket,
            envelope,
            &self.context,
            &self.room,
            &self.routes,
        )
    }
    fn jobs(&self) -> Vec<ForwardJob> {
        self.helper.take_ready(&self.actor, &self.routes).unwrap()
    }
    fn update(&mut self, epoch: u64, seeds: &[u8]) {
        self.actor.sync_view(view(epoch, seeds)).unwrap();
        self.context.epoch = epoch;
    }
}

#[test]
fn voice_transport_helper_cached_receipt_keeps_first_actor_and_room_incarnations() {
    for replacement in 0..2 {
        let mut f = Fixture::new();
        let envelope = f.targeted(76);
        let receipt = f.receipt(envelope.clone()).unwrap();
        let before = f.helper.counts();
        assert!(receipt.authorized());
        if replacement == 0 {
            let (mut actor, registration) = f
                .runtime
                .prepare_actor(f.context.scope.group, id(2), &f.runtime.session_ticket())
                .unwrap();
            registration.register().unwrap();
            actor
                .sync_view(view(7, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]))
                .unwrap();
            f.actor = actor;
        } else {
            f.room.lease.revoke();
            f.room = RoomAuthority {
                scope: f.context.scope,
                lease: CallLease::default(),
                custody: None,
            };
        }
        assert!(!receipt.authorized());
        assert_eq!(f.receipt(envelope).err(), Some(VoiceError::Unauthorized));
        assert_eq!(
            f.helper.counts(),
            before,
            "replacement cannot refill edges or attempts"
        );
    }
}

#[test]
fn voice_transport_helper_cached_receipt_cannot_extend_first_short_permit() {
    let f = Fixture::new();
    let object = f.targeted(77).object().clone();
    let first = RoutePermit::issue(&signer(1), &object, id(2), 1000, 2000).unwrap();
    let receipt = f
        .receipt(Envelope::ToHelper {
            object: object.clone(),
            permit: first,
        })
        .unwrap();
    f.clock.advance_ms(6001); // Past first permit plus skew, while the original object is fresh.
    assert!(!receipt.authorized());
    let before = f.helper.counts();
    let later = RoutePermit::issue(&signer(1), &object, id(2), 7001, 16_000).unwrap();
    assert_eq!(
        f.receipt(Envelope::ToHelper {
            object,
            permit: later
        })
        .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.helper.counts(), before);
    assert!(
        f.jobs().is_empty(),
        "a later permit cannot revive first-permit work"
    );
}

#[test]
fn voice_transport_helper_room_receipt_has_original_authority_with_zero_or_partial_edges() {
    for empty in [false, true] {
        let mut f = Fixture::new();
        if empty {
            f.routes.clear();
        }
        let envelope = f.announcement(78, 1);
        let receipt = f.receipt(envelope.clone()).unwrap();
        assert_eq!(receipt.info().edges == 0, empty);
        if !empty {
            // A room admission is not an all-recipients delivery claim. Removing one edge
            // must not invalidate custody for other targets (the queued reply's epoch/route
            // gate is enforced separately by the actual transport response owner).
            f.update(8, &[1, 2, 4, 5, 6, 7, 8, 9, 10]);
            assert!(receipt.authorized());
        }
        f.room.lease.revoke();
        assert!(!receipt.authorized());
        f.room = RoomAuthority {
            scope: f.context.scope,
            lease: CallLease::default(),
            custody: None,
        };
        assert_eq!(f.receipt(envelope).err(), Some(VoiceError::Unauthorized));
    }
}

#[test]
fn voice_transport_ticket_replaces_requester_debit_and_rejects_frame_substitution() {
    let f = Fixture::new();
    let packet = f.targeted(81);
    let bytes = packet.encode();
    for _ in 0..40 {
        let ticket = f.actor.admit_transport_verification(id(1), &bytes).unwrap();
        f.helper
            .admit_verified(
                &f.actor,
                ticket,
                packet.clone(),
                &f.context,
                &f.room,
                &f.routes,
            )
            .unwrap();
    }
    assert_eq!(
        f.actor.admit_transport_verification(id(1), &bytes).err(),
        Some(VoiceError::RateLimited),
        "one requester debit per exact frame, not two"
    );
    assert_eq!(
        f.helper.counts().0,
        1,
        "all repeats share one original packet owner"
    );
    let f = Fixture::new();
    let ticket = f
        .actor
        .admit_transport_verification(id(1), &f.targeted(82).encode())
        .unwrap();
    assert_eq!(
        f.helper
            .admit_verified(
                &f.actor,
                ticket,
                f.targeted(83),
                &f.context,
                &f.room,
                &f.routes
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.helper.counts().0, 0);
    for renewal in 1..=8 {
        f.admit(f.announcement(90 + renewal as u8, renewal))
            .unwrap();
    }
}

#[derive(Debug)]
struct IngressPauseClock {
    inner: Arc<ManualClock>,
    observed: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}
impl catcoms_rt::Clock for IngressPauseClock {
    fn now_ms(&self) -> u64 {
        let now = self.inner.now_ms();
        if let Some(sender) = self.observed.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
        now
    }
    fn monotonic_ms(&self) -> u64 {
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
fn voice_transport_expiry_while_waiting_for_helper_lock_commits_no_origin_packet_or_cursor() {
    let manual = Arc::new(ManualClock::new(1000));
    let clock = Arc::new(IngressPauseClock {
        inner: manual.clone(),
        observed: Mutex::new(None),
    });
    let f = Fixture::on(Runtime::new(clock.clone()), manual.clone(), scope());
    let packet = f.announcement(80, 1);
    let ticket = f
        .actor
        .admit_transport_verification(id(1), &packet.encode())
        .unwrap();
    let held = f.helper.state.lock().unwrap();
    let (entered, seen) = std::sync::mpsc::channel();
    *clock.observed.lock().unwrap() = Some(entered);
    std::thread::scope(|threads| {
        let worker = threads.spawn(|| {
            f.helper
                .admit_verified(&f.actor, ticket, packet, &f.context, &f.room, &f.routes)
        });
        // Route classification has passed its initial ticket check. The held helper mutex
        // prevents the final capacity/origin transaction until after the admission expires.
        seen.recv().unwrap();
        manual.advance_ms(5_000);
        drop(held);
        assert_eq!(worker.join().unwrap().err(), Some(VoiceError::Unauthorized));
    });
    assert_eq!(f.helper.counts(), (0, 0, 0));
    assert!(f.helper.state.lock().unwrap().cursors.is_empty());
    // Original announcement quota/key ownership was never installed by the expired frame.
    for renewal in 1..=8 {
        f.admit(f.announcement(90 + renewal as u8, renewal))
            .unwrap();
    }
    assert_eq!(
        f.admit(f.announcement(100, 9)).err(),
        Some(VoiceError::RateLimited)
    );
}

#[test]
fn voice_transport_original_request_retirement_while_waiting_for_helper_lock_refuses_new_and_cached_admission(
) {
    for cached in [false, true] {
        let manual = Arc::new(ManualClock::new(1000));
        let clock = Arc::new(IngressPauseClock {
            inner: manual.clone(),
            observed: Mutex::new(None),
        });
        let f = Fixture::on(Runtime::new(clock.clone()), manual, scope());
        let packet = f.targeted(79);
        if cached {
            f.admit(packet.clone()).unwrap();
        }
        let before = f.helper.counts();
        let (responder, receiver) = catcoms_rt::Responder::channel();
        let original = responder.cancellation_authority();
        let ticket = f
            .actor
            .admit_transport_verification(id(1), &packet.encode())
            .unwrap()
            .bind_request(original.clone())
            .unwrap();
        let held = f.helper.state.lock().unwrap();
        let (entered, seen) = std::sync::mpsc::channel();
        *clock.observed.lock().unwrap() = Some(entered);
        std::thread::scope(|threads| {
            let worker = threads.spawn(|| {
                f.helper.admit_verified(
                    &f.actor,
                    ticket,
                    packet.clone(),
                    &f.context,
                    &f.room,
                    &f.routes,
                )
            });
            seen.recv().unwrap();
            drop(receiver);
            assert!(!original.authorized());
            drop(held);
            assert_eq!(
                worker.join().unwrap().err(),
                Some(VoiceError::Unauthorized),
                "cached={cached}"
            );
        });
        assert_eq!(
            f.helper.counts(),
            before,
            "canceled old ingress creates no original packet or fresh edge allowance"
        );
        // Fresh original ingress remains usable; cancellation did not burn origin eligibility.
        f.admit(packet).unwrap();
        drop(responder);
    }
}

#[test]
fn voice_transport_tickets_refuse_epoch_replacement_and_hold_actual_credit_after_lock() {
    let mut f = Fixture::new();
    let bytes = f.targeted(84).encode();
    let ticket = f.actor.admit_transport_verification(id(1), &bytes).unwrap();
    f.update(8, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(
        ticket.check(&f.actor, &bytes),
        Err(VoiceError::Unauthorized)
    );
    drop(ticket);
    let ticket = f.actor.admit_transport_verification(id(1), &bytes).unwrap();
    let before = f.runtime.resources().counts();
    f.runtime.revoke_session();
    assert_eq!(
        ticket.check(&f.actor, &bytes),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.runtime.resources().counts(),
        before,
        "revocation is not actual crypto settlement"
    );
    drop(ticket);
    assert!(f.runtime.resources().counts().0 < before.0);
}

#[test]
fn voice_transport_verification_process_rate_survives_groups_and_actor_replacement() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let mut groups = (0..4)
        .map(|index| {
            let mut group_scope = scope();
            group_scope.group = [index + 1; 32];
            Fixture::on(runtime.clone(), clock.clone(), group_scope)
        })
        .collect::<Vec<_>>();
    for group in &groups {
        for _ in 0..32 {
            drop(
                group
                    .actor
                    .admit_transport_verification(id(1), b"bounded frame")
                    .unwrap(),
            );
        }
    }
    let first = &mut groups[0];
    let (mut replacement, registration) = runtime
        .prepare_actor(first.context.scope.group, id(2), &runtime.session_ticket())
        .unwrap();
    registration.register().unwrap();
    replacement
        .sync_view(view(7, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]))
        .unwrap();
    first.actor = replacement;
    assert_eq!(
        first
            .actor
            .admit_transport_verification(id(3), b"bounded frame")
            .err(),
        Some(VoiceError::RateLimited)
    );
    clock.advance_ms(1_000);
    assert!(first
        .actor
        .admit_transport_verification(id(3), b"bounded frame")
        .is_ok());
}

#[test]
fn original_packet_and_final_identity_survive_opaque_helper_forwarding() {
    let f = Fixture::new();
    let incoming = f.targeted(10);
    let original = incoming.object().clone();
    assert_eq!(
        f.admit(incoming).unwrap(),
        Admission {
            cached: false,
            edges: 1
        }
    );
    let mut jobs = f.jobs();
    assert_eq!(jobs.len(), 1);
    let job = jobs.pop().unwrap();
    assert_eq!(job.target(), id(3));
    assert_eq!(job.peer(), PeerId::from_u64(3));
    assert_eq!(job.envelope().object().encode(), original.encode());
    let recipient = Context {
        local: id(3),
        ..f.context
    };
    assert_eq!(
        job.envelope().authorize(id(2), &recipient, |_| true),
        Ok(Disposition::Deliver)
    );
    assert!(matches!(job.envelope(), Envelope::HelperDelivery { .. }));
    f.helper.complete(job.finish(true)).unwrap();
    assert!(f.jobs().is_empty());
    assert_eq!(f.helper.counts(), (1, 1, 0));
}

#[test]
fn cached_retries_spend_requester_work_without_exhausting_new_origin_allowance() {
    let f = Fixture::new();
    let first = f.announcement(10, 1);
    f.admit(first.clone()).unwrap();
    for _ in 0..20 {
        assert!(f.admit(first.clone()).unwrap().cached);
    }
    f.admit(f.announcement(11, 2)).unwrap();
    assert!(
        f.admit(first.clone()).unwrap().cached,
        "cached old renewal returns only the original status"
    );
    for renewal in 3..=8 {
        assert!(
            !f.admit(f.announcement(renewal as u8 + 10, renewal))
                .unwrap()
                .cached
        );
    }
    assert_eq!(
        f.admit(f.announcement(19, 9)).err(),
        Some(VoiceError::RateLimited)
    );
    // 1 original + 20 retries + 1 newer + 1 cached old + 6 newer + 1 refusal = 30 requester calls.
    for _ in 30..40 {
        assert!(f.admit(first.clone()).unwrap().cached);
    }
    assert_eq!(
        f.admit(first).err(),
        Some(VoiceError::RateLimited),
        "duplicates cannot refund requester throttling"
    );
}

#[test]
fn rejected_stale_new_packet_restores_origin_allowance_and_binding_deadline() {
    let f = Fixture::new();
    f.admit(f.announcement(10, 1)).unwrap();
    f.admit(f.announcement(11, 2)).unwrap();
    let before = f
        .actor
        .with_limits(|limits| Ok(limits.delegation_deadline(id(1), key().join)))
        .unwrap();
    f.clock.advance_ms(1000);
    let stale = f.announcement(12, 1);
    for _ in 0..12 {
        assert_eq!(f.admit(stale.clone()).err(), Some(VoiceError::Freshness));
        assert_eq!(
            f.actor
                .with_limits(|limits| Ok(limits.delegation_deadline(id(1), key().join)))
                .unwrap(),
            before,
            "failed renewal must not extend original key protection"
        );
    }
    // One second refilled the room bucket to eight; none of the failed admissions consumed it.
    for renewal in 3..=10 {
        f.admit(f.announcement(renewal as u8 + 20, renewal))
            .unwrap();
    }
    assert_eq!(
        f.admit(f.announcement(40, 11)).err(),
        Some(VoiceError::RateLimited)
    );
}

#[test]
fn absent_target_keeps_one_bounded_edge_until_a_proven_connected_route_returns() {
    let mut f = Fixture::new();
    f.routes.retain(|route| route.device != id(3));
    let packet = f.targeted(10);
    assert_eq!(
        f.admit(packet.clone()).unwrap(),
        Admission {
            cached: false,
            edges: 1
        }
    );
    assert!(f.jobs().is_empty());
    assert_eq!(f.helper.counts(), (1, 1, 0));
    assert!(f.admit(packet).unwrap().cached);
    f.routes.push(BoundRoute {
        device: id(3),
        peer: PeerId::from_u64(30),
    });
    let job = f.jobs().pop().unwrap();
    assert_eq!(job.peer(), PeerId::from_u64(30));
    assert_eq!(
        job.flight.attempt, 1,
        "waiting consumes no lower work allowance"
    );
}

#[test]
fn duplicates_changed_permits_and_lost_waiters_never_restore_edge_attempts() {
    let f = Fixture::new();
    let incoming = f.targeted(10);
    f.admit(incoming.clone()).unwrap();
    for attempt in 1..=3 {
        let mut jobs = f.jobs();
        assert_eq!(jobs.len(), 1);
        let job = jobs.pop().unwrap();
        assert_eq!(job.flight.attempt, attempt);
        let lower = job.keepalive();
        f.helper.complete(job.finish(false)).unwrap();
        assert!(
            f.jobs().is_empty(),
            "a lower holder must drain before any retry"
        );
        assert!(f.admit(incoming.clone()).unwrap().cached);
        drop(lower);
        // Contract (voice_schedule review): a failed edge also waits its retry gap, so a
        // scheduler pumping after every completion cannot send all attempts back to back.
        if attempt < 3 {
            assert!(f.jobs().is_empty(), "the retry gap holds the next attempt");
            f.clock.advance_ms(500);
        }
    }
    assert!(f.jobs().is_empty());
    let mut changed = incoming;
    if let Envelope::ToHelper { object, permit } = &mut changed {
        *permit = RoutePermit::issue(&signer(1), object, id(2), 1001, 16_000).unwrap();
    }
    assert!(f.admit(changed).unwrap().cached);
    assert!(f.jobs().is_empty());
}

#[test]
fn lost_completion_still_spends_lower_attempt_and_real_driver_holds_credit() {
    let f = Fixture::new();
    f.admit(f.targeted(10)).unwrap();
    let before = f.runtime.resources().counts().0;
    let job = f.jobs().pop().unwrap();
    let actual = f.runtime.resources().counts().0;
    assert!(actual > before);
    let lower = job.keepalive();
    drop(job); // Simulates lost actor completion, not lower completion.
    assert!(f.jobs().is_empty());
    assert_eq!(f.runtime.resources().counts().0, actual);
    drop(lower);
    assert_eq!(f.runtime.resources().counts().0, before);
    let retry = f.jobs().pop().unwrap();
    assert_eq!(retry.flight.attempt, 2);
}

#[tokio::test]
async fn stale_queued_member_room_policy_lock_and_actor_effects_never_revive() {
    for action in 0..5 {
        let mut f = Fixture::new();
        f.admit(f.targeted(10)).unwrap();
        let job = f.jobs().pop().unwrap();
        let lower = job.keepalive();
        let (_cancel, receiver) = watch::channel(false);
        let mut queued =
            RequestCancellation::new(receiver, Some(lower.clone())).with_authority(job.authority());
        let before = f.runtime.resources().counts().0;
        match action {
            0 => {
                f.update(8, &[1, 2, 4, 5, 6, 7, 8, 9, 10]);
                f.update(9, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
            }
            1 => {
                f.update(8, &[2, 3, 4, 5, 6, 7, 8, 9, 10]);
                f.update(9, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
            }
            2 => f.room.lease.revoke(),
            3 => {
                let mut changed = view(8, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
                changed.indirect = false;
                f.actor.sync_view(changed).unwrap();
            }
            _ => f.runtime.revoke_session(),
        }
        assert!(queued.is_cancelled());
        queued.cancelled().await;
        f.helper.complete(job.finish(true)).unwrap();
        assert_eq!(f.helper.counts(), (1, 1, 1));
        assert!(f.runtime.resources().counts().0 >= before);
        drop(queued);
        drop(lower);
        if action < 3 {
            assert!(f.jobs().is_empty());
        }
    }
}

#[test]
fn helper_actor_replacement_keeps_protection_but_refuses_fresh_custody_ack_for_retired_packet() {
    let mut f = Fixture::new();
    let packet = f.targeted(10);
    f.admit(packet.clone()).unwrap();
    let job = f.jobs().pop().unwrap();
    f.helper.complete(job.finish(false)).unwrap();
    let ticket = f.runtime.session_ticket();
    let (mut new, registration) = f
        .runtime
        .prepare_actor(scope().group, id(2), &ticket)
        .unwrap();
    registration.register().unwrap();
    new.sync_view(view(7, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]))
        .unwrap();
    f.actor = new;
    assert!(Arc::ptr_eq(&f.helper, &f.actor.helper().unwrap()));
    // Contract correction: retained bytes still protect the original identity/budget, but
    // their first actor is retired. Returning a current HelperQueued receipt would falsely
    // claim actionable custody under the new actor; a duplicate must now refuse.
    let before = f.helper.counts();
    assert_eq!(f.admit(packet).err(), Some(VoiceError::Unauthorized));
    assert_eq!(f.helper.counts(), before);
    assert!(f.jobs().is_empty());
    // Fresh packet can use this actor; old immutable cache was not forgotten or re-forwarded.
    f.admit(f.targeted(11)).unwrap();
    assert_eq!(f.jobs().len(), 1);
}

#[test]
fn quiet_room_recipients_get_turns_and_duplicates_cannot_expand_fanout() {
    let f = Fixture::new();
    let mut seen = BTreeSet::new();
    for renewal in 1..=2 {
        let packet = f.announcement(renewal as u8, renewal);
        assert_eq!(f.admit(packet.clone()).unwrap().edges, 4);
        assert!(f.admit(packet).unwrap().cached);
        let jobs = f.jobs();
        assert_eq!(jobs.len(), 4);
        for job in jobs {
            assert_ne!(job.target(), id(1));
            assert_ne!(job.target(), id(2));
            seen.insert(job.target());
            f.helper.complete(job.finish(true)).unwrap();
        }
    }
    assert_eq!(
        seen.len(),
        8,
        "rotation must cover members beyond the first four"
    );
    assert_eq!(f.helper.counts(), (2, 8, 0));
}

#[test]
fn wrong_roles_scope_directory_lease_routes_and_policy_refuse_without_forwarding() {
    let mut f = Fixture::new();
    let packet = f.targeted(10);
    let direct = Envelope::Direct(packet.object().clone());
    assert_eq!(f.admit(direct).err(), Some(VoiceError::Unauthorized));
    assert!(f
        .helper
        .admit(
            &f.actor,
            id(3),
            packet.clone(),
            &f.context,
            &f.room,
            &f.routes
        )
        .is_err());
    let wrong = RoomAuthority {
        scope: Scope {
            room: 999,
            ..scope()
        },
        lease: CallLease::default(),
        custody: None,
    };
    assert!(f
        .helper
        .admit(
            &f.actor,
            id(1),
            packet.clone(),
            &f.context,
            &wrong,
            &f.routes
        )
        .is_err());
    let mut duplicate_route = f.routes.clone();
    duplicate_route.push(duplicate_route[0]);
    assert_eq!(
        f.helper
            .admit(
                &f.actor,
                id(1),
                packet.clone(),
                &f.context,
                &f.room,
                &duplicate_route
            )
            .err(),
        Some(VoiceError::Malformed)
    );
    let mut changed = view(8, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    changed.indirect = false;
    f.actor.sync_view(changed).unwrap();
    f.context.epoch = 8;
    f.context.indirect = false;
    assert_eq!(f.admit(packet).err(), Some(VoiceError::Unauthorized));
    assert_eq!(f.helper.counts(), (0, 0, 0));
}

#[test]
fn changed_object_with_reused_identity_cannot_replace_retained_packet() {
    let f = Fixture::new();
    f.admit(f.targeted(10)).unwrap();
    let changed = SignedObject::issue(
        scope(),
        &signer(1),
        [10; 32],
        1000,
        16_000,
        Body::WatchChallenge {
            watcher: [88; 32],
            target: id(3),
            join: [3; 32],
            key_ref: [4; 32],
            challenge: [5; 32],
        },
    )
    .unwrap();
    let permit = RoutePermit::issue(&signer(1), &changed, id(2), 1000, 16_000).unwrap();
    assert_eq!(
        f.admit(Envelope::ToHelper {
            object: changed,
            permit
        })
        .err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(f.jobs().len(), 1);
}

#[test]
fn permit_expiry_stops_forwarding_before_longer_announcement_expiry() {
    let f = Fixture::new();
    f.admit(f.announcement(10, 1)).unwrap();
    f.clock.advance_ms(20_001);
    assert!(f.jobs().is_empty());
    assert_eq!(
        f.helper.counts().0,
        1,
        "original object remains protected after route permission expires"
    );
}

#[test]
fn expired_records_wait_for_real_lower_drain_and_rollback_cannot_replay_them() {
    let f = Fixture::new();
    let packet = f.targeted(10);
    f.admit(packet.clone()).unwrap();
    let job = f.jobs().pop().unwrap();
    let lower = job.keepalive();
    drop(job);
    f.clock.advance_ms(25_001);
    assert_eq!(f.helper.counts(), (1, 1, 1));
    drop(lower);
    assert_eq!(f.helper.counts(), (0, 0, 0));
    f.clock.set_wall_ms(1000);
    assert_eq!(f.admit(packet).err(), Some(VoiceError::Freshness));
}

#[test]
fn rollback_before_expiry_cannot_resurrect_pruned_delegation_or_conflicting_join_key() {
    let f = Fixture::new();
    let packet = f.announcement(10, 2);
    f.admit(packet.clone()).unwrap();
    f.clock.advance_ms(61_000);
    f.clock.set_wall_ms(500);
    assert_eq!(f.helper.counts(), (0, 0, 0));
    // Age out the original monotonic limit record, exactly the rollback-before-expiry boundary.
    f.actor
        .with_limits(|limits| limits.requester(id(4), f.clock.monotonic_ms()))
        .unwrap();
    assert_eq!(f.admit(packet).err(), Some(VoiceError::Freshness));
    // Negated P-256 generator: a different valid curve point for the same old join.
    let point = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296b01cbd1c01e58065711814b583f061e9d431cca994cea1313449bf97c840ae0a").unwrap();
    let changed_key = JoinKey::new([1; 32], point.try_into().unwrap()).unwrap();
    let changed = SignedObject::issue(
        scope(),
        &signer(1),
        [11; 32],
        1000,
        31_000,
        Body::Announcement {
            key: changed_key,
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap();
    let permit = RoutePermit::issue(&signer(1), &changed, id(2), 1000, 16_000).unwrap();
    assert_eq!(
        f.admit(Envelope::ToHelper {
            object: changed,
            permit
        })
        .err(),
        Some(VoiceError::Freshness)
    );
}

#[test]
fn eight_process_forward_slots_follow_real_lower_holders_across_distinct_groups() {
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let first = Fixture::on(runtime.clone(), clock.clone(), scope());
    let second = Fixture::on(
        runtime.clone(),
        clock.clone(),
        Scope::for_group(b"second native group", scope().policy, 123),
    );
    let third = Fixture::on(
        runtime,
        clock,
        Scope::for_group(b"third native group", scope().policy, 123),
    );
    first.admit(first.announcement(10, 1)).unwrap();
    second.admit(second.announcement(10, 1)).unwrap();
    third.admit(third.targeted(10)).unwrap();
    let mut first_jobs = first.jobs();
    let second_jobs = second.jobs();
    assert_eq!(first_jobs.len() + second_jobs.len(), 8);
    assert!(third.jobs().is_empty());
    let job = first_jobs.pop().unwrap();
    let lower = job.keepalive();
    drop(job);
    assert!(
        third.jobs().is_empty(),
        "cancelled upper work is still charged on the actual lower driver"
    );
    drop(lower);
    let job = third.jobs().pop().unwrap();
    assert_eq!(job.flight.attempt, 1);
    assert_eq!(
        first.helper.counts().2 + second.helper.counts().2 + third.helper.counts().2,
        8
    );
}

#[test]
fn capacity_refusal_during_edge_or_cursor_charging_commits_no_partial_packet_or_rotation() {
    for at_cursor in [false, true] {
        let f = Fixture::new();
        f.admit(f.announcement(10, 1)).unwrap();
        let cursor_before = f.helper.state.lock().unwrap().cursors[&(id(1), 123)].last;
        let room_id = if at_cursor { 124 } else { 123 };
        let native = Context {
            scope: Scope {
                room: room_id,
                ..f.context.scope
            },
            ..f.context
        };
        let room = RoomAuthority {
            scope: native.scope,
            lease: CallLease::default(),
            custody: None,
        };
        let object = SignedObject::issue(
            native.scope,
            &signer(1),
            [11; 32],
            1000,
            31_000,
            Body::Announcement {
                key: key(),
                epoch: 7,
                renewal: 2,
            },
        )
        .unwrap();
        let permit = RoutePermit::issue(&signer(1), &object, id(2), 1000, 16_000).unwrap();
        let packet = Envelope::ToHelper { object, permit };
        let packet_charge = packet.encode().len() + 1024;
        let persistent_remaining = packet_charge + 1024 * if at_cursor { 4 } else { 2 };
        let current = f.runtime.resources().counts().0;
        let fill = f
            .runtime
            .resources()
            .charge(
                scope().group,
                256 * 1024 - current - persistent_remaining,
                ResourceKind::Retained,
            )
            .unwrap();
        let full_before = f.runtime.resources().counts();
        for _ in 0..10 {
            assert_eq!(
                f.helper
                    .admit(&f.actor, id(1), packet.clone(), &native, &room, &f.routes)
                    .err(),
                Some(VoiceError::Capacity)
            );
        }
        assert_eq!(
            f.runtime.resources().counts(),
            full_before,
            "all staged persistent credits return after failed transaction"
        );
        assert_eq!(f.helper.counts(), (1, 4, 0));
        {
            let state = f.helper.state.lock().unwrap();
            assert_eq!(state.cursors.len(), 1);
            assert_eq!(state.cursors[&(id(1), 123)].last, cursor_before);
        }
        drop(fill);
        assert_eq!(
            f.helper
                .admit(&f.actor, id(1), packet, &native, &room, &f.routes)
                .unwrap(),
            Admission {
                cached: false,
                edges: 4
            }
        );
        assert_eq!(f.helper.counts(), (2, 8, 0));
        if at_cursor {
            assert_eq!(f.helper.state.lock().unwrap().cursors.len(), 2);
        } else {
            assert_ne!(
                f.helper.state.lock().unwrap().cursors[&(id(1), 123)].last,
                cursor_before
            );
        }
        for renewal in 3..=8 {
            f.admit(f.announcement(renewal as u8 + 10, renewal))
                .unwrap();
        }
        assert_eq!(
            f.admit(f.announcement(40, 9)).err(),
            Some(VoiceError::RateLimited),
            "ten failed capacity transactions spent no original room tokens"
        );
    }
}

#[test]
fn process_forward_slots_are_shared_and_busy_edges_preserve_attempt_allowance() {
    let f = Fixture::new();
    f.admit(f.announcement(10, 1)).unwrap();
    let jobs = f.jobs();
    assert_eq!(jobs.len(), 4);
    f.admit(f.targeted(11)).unwrap();
    assert!(f.jobs().is_empty());
    drop(jobs);
    let jobs = f.jobs();
    assert!(jobs
        .iter()
        .any(|job| job.packet.envelope.object().id() == [11; 32] && job.flight.attempt == 1));
    assert!(jobs.len() <= 4);
}
