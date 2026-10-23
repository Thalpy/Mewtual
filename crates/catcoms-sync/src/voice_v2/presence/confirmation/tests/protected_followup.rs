//! Complete protected confirmation, with real native signatures and original actor custody.
//! Only AEAD output is mocked. This is not crypto interoperability, IPC or audible acceptance.
use super::*;
use crate::voice_v2::chain::ConfirmationAdmission;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::{ActorOwner, Runtime, View};
use rand_core::RngCore;
mod derivation;
mod observation;
mod result;

struct ProtectedFixture {
    f: Fixture,
    _runtimes: [Arc<Runtime>; 2],
    low: ActorOwner,
    high: ActorOwner,
    origin_clock: Arc<OriginClock>,
}

#[derive(Debug)]
struct OriginClock {
    inner: ManualClock,
    reads: std::sync::atomic::AtomicUsize,
    revoke: std::sync::Mutex<Option<(usize, CallLease)>>,
    advance: std::sync::Mutex<Option<(usize, u64)>>,
}
impl Clock for OriginClock {
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        let read = self.reads.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
        let mut revoke = self.revoke.lock().unwrap();
        if revoke.as_ref().is_some_and(|(at, _)| *at == read) {
            revoke.take().unwrap().1.revoke();
        }
        let mut advance = self.advance.lock().unwrap();
        if advance.as_ref().is_some_and(|(at, _)| *at == read) {
            self.inner.advance_ms(advance.take().unwrap().1);
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
fn register(runtime: &Arc<Runtime>, end: &End, peer: DeviceId) -> ActorOwner {
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(end.context.scope.group, end.context.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: end.context.epoch,
            members: [end.context.local, peer].into(),
            policy: end.context.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    actor
}
fn fixture() -> ProtectedFixture {
    let clock = ManualClock::new(1000);
    let origin_clock = Arc::new(OriginClock {
        inner: clock.clone(),
        reads: std::sync::atomic::AtomicUsize::new(0),
        revoke: std::sync::Mutex::new(None),
        advance: std::sync::Mutex::new(None),
    });
    let a_rt = Runtime::new(origin_clock.clone());
    let b_rt = Runtime::new(origin_clock.clone());
    let mut rng = ChaCha20Rng::from_seed([99; 32]);
    let a = End::new(1, a_rt.resources(), &mut rng);
    let b = End::new(2, b_rt.resources(), &mut rng);
    let (mut low, mut high, low_rt, high_rt) = if a.context.local < b.context.local {
        (a, b, a_rt, b_rt)
    } else {
        (b, a, b_rt, a_rt)
    };
    let id = admit_pair(&mut low, &mut high, &clock, &mut rng);
    let low_actor = register(&low_rt, &low, high.context.local);
    let high_actor = register(&high_rt, &high, low.context.local);
    for end in [&mut low, &mut high] {
        end.join
            .prepare_crypto_chain(id, &ca(&end.context, &end.lease), &clock)
            .unwrap();
    }
    let pool = low_rt.resources();
    ProtectedFixture {
        f: Fixture {
            low,
            high,
            id,
            clock,
            rng,
            pool,
        },
        _runtimes: [low_rt, high_rt],
        low: low_actor,
        high: high_actor,
        origin_clock,
    }
}
fn room(end: &End) -> RoomAuthority {
    RoomAuthority {
        scope: end.context.scope,
        lease: end.lease.clone(),
        custody: None,
    }
}
fn start(p: &mut ProtectedFixture, predecessor: Option<Nonce>) -> Arc<CryptoAction> {
    let end = &mut p.f.low;
    let room = room(end);
    end.join
        .start_protected_hello(
            ProtectedStart {
                actor: &p.low,
                room: &room,
                handshake: p.f.id,
                predecessor,
            },
            &ca(&end.context, &end.lease),
            Arc::new(p.f.clock.clone()),
            &mut p.f.rng,
            |_, _| Ok(()),
        )
        .unwrap()
}
fn respond(
    end: &mut End,
    actor: &ActorOwner,
    id: Nonce,
    trigger: &Arc<CryptoAction>,
    clock: &ManualClock,
    rng: &mut ChaCha20Rng,
) -> Result<Arc<CryptoAction>, VoiceError> {
    let room = room(end);
    end.join.respond_protected_confirmation(
        ProtectedResponse {
            actor,
            room: &room,
            handshake: id,
            trigger,
        },
        &ca(&end.context, &end.lease),
        Arc::new(clock.clone()),
        rng,
        |_, _| Ok(()),
    )
}
fn admit(
    end: &mut End,
    actor: &ActorOwner,
    object: SignedObject,
    clock: &ManualClock,
) -> (Arc<ConfirmationAdmission>, Arc<CryptoAction>) {
    try_admit(end, actor, object, clock).unwrap()
}
fn try_admit(
    end: &mut End,
    actor: &ActorOwner,
    object: SignedObject,
    clock: &ManualClock,
) -> Result<(Arc<ConfirmationAdmission>, Arc<CryptoAction>), VoiceError> {
    let room = room(end);
    let envelope = Envelope::Direct(object);
    let ticket =
        actor.admit_transport_verification(envelope.object().issuer(), &envelope.encode())?;
    let receipt = end.join.admit_protected_confirmation(
        actor,
        &ticket,
        &envelope,
        &room,
        &ca(&end.context, &end.lease),
        Arc::new(clock.clone()),
        |_, _| Ok(()),
    )?;
    let action = end.join.actions.get(Key::Decrypt(receipt.token)).unwrap();
    Ok((receipt, action))
}
fn open(
    end: &mut End,
    action: &Arc<CryptoAction>,
    bytes: &[u8],
    clock: &ManualClock,
    rng: &mut ChaCha20Rng,
) -> CryptoResultReceipt {
    action.admit_engine(action.engine_bytes()).unwrap();
    let result = end
        .join
        .commit_confirmation_result(
            action,
            bytes,
            action.output_tag(bytes).unwrap(),
            &ca(&end.context, &end.lease),
            clock,
            rng,
        )
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&result).unwrap();
    result
}
fn seal(end: &mut End, action: &Arc<CryptoAction>, clock: &ManualClock) -> (SignedObject, Vec<u8>) {
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("native seal");
    };
    let bytes = flight.plaintext.clone();
    action.admit_engine(action.engine_bytes()).unwrap();
    let object = end
        .join
        .submit_confirmation_action(
            action,
            &vec![22; bytes.len() + 16],
            &end.signer,
            &ca(&end.context, &end.lease),
            clock,
        )
        .unwrap();
    let result = action.terminal_status().receipt.unwrap();
    action.settle_engine();
    action.acknowledge_result(&result).unwrap();
    (object, bytes)
}
fn message(action: &CryptoAction) -> Nonce {
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("native seal");
    };
    flight.reservation.id
}

#[test]
fn protected_all_three_flights_progress_and_recover_exact_factories_after_input_compaction() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, plain) = seal(&mut p.f.low, &hello, &p.f.clock);
    let (hello_receipt, high_open) = admit(&mut p.f.high, &p.high, object.clone(), &p.f.clock);
    assert_eq!(
        open(&mut p.f.high, &high_open, &plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Opened
    );
    assert!(high_open.input().is_err());
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &high_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let before = p.f.rng.clone().next_u64();
    assert!(Arc::ptr_eq(
        &proof,
        &respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &high_open,
            &p.f.clock,
            &mut p.f.rng
        )
        .unwrap()
    ));
    assert_eq!(p.f.rng.clone().next_u64(), before);
    assert!(Arc::ptr_eq(
        &hello_receipt,
        &admit(&mut p.f.high, &p.high, object, &p.f.clock).0
    ));
    let (object, plain) = seal(&mut p.f.high, &proof, &p.f.clock);
    let (_, low_open) = admit(&mut p.f.low, &p.low, object.clone(), &p.f.clock);
    assert_eq!(
        open(&mut p.f.low, &low_open, &plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Opened
    );
    let finish = respond(
        &mut p.f.low,
        &p.low,
        p.f.id,
        &low_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (finish_object, finish_plain) = seal(&mut p.f.low, &finish, &p.f.clock);
    assert!(p.f.low.established(p.f.id, &p.f.clock));
    let (_, high_finish) = admit(&mut p.f.high, &p.high, finish_object, &p.f.clock);
    assert_eq!(
        open(
            &mut p.f.high,
            &high_finish,
            &finish_plain,
            &p.f.clock,
            &mut p.f.rng
        )
        .outcome,
        CryptoOutcome::Opened
    );
    assert!(p.f.high.established(p.f.id, &p.f.clock));
    // Historical factory recovery is independent of established state and consumed input.
    assert!(Arc::ptr_eq(
        &finish,
        &respond(
            &mut p.f.low,
            &p.low,
            p.f.id,
            &low_open,
            &p.f.clock,
            &mut p.f.rng
        )
        .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &proof,
        &respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &high_open,
            &p.f.clock,
            &mut p.f.rng
        )
        .unwrap()
    ));
    assert!(proof.input().is_err());
    assert!(finish.input().is_err());
}

#[test]
fn protected_confirmation_completes_under_full_ordinary_pool_pressure_at_both_endpoints() {
    let mut p = fixture();
    // The native verification tickets are ordinary work, not covered by confirmation's
    // workflow. Prepay each real ticket before filling unrelated ordinary capacity.
    let hello = start(&mut p, None);
    let (hello_object, hello_plain) = seal(&mut p.f.low, &hello, &p.f.clock);
    let (_, hello_open) = admit(&mut p.f.high, &p.high, hello_object, &p.f.clock);
    let high_pool = p._runtimes[1].resources();
    let high_full = high_pool
        .charge(
            p.f.high.context.scope.group,
            crate::voice_v2::resources::GROUP_BYTES - high_pool.counts().0,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        open(
            &mut p.f.high,
            &hello_open,
            &hello_plain,
            &p.f.clock,
            &mut p.f.rng
        )
        .outcome,
        CryptoOutcome::Opened
    );
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &hello_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, plain) = seal(&mut p.f.high, &proof, &p.f.clock);
    let (_, proof_open) = admit(&mut p.f.low, &p.low, object, &p.f.clock);
    let low_full =
        p.f.pool
            .charge(
                p.f.low.context.scope.group,
                crate::voice_v2::resources::GROUP_BYTES - p.f.pool.counts().0,
                crate::voice_v2::resources::ResourceKind::Retained,
            )
            .unwrap();
    assert_eq!(
        open(&mut p.f.low, &proof_open, &plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Opened
    );
    let finish = respond(
        &mut p.f.low,
        &p.low,
        p.f.id,
        &proof_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, plain) = seal(&mut p.f.low, &finish, &p.f.clock);
    drop(high_full); // A fresh authenticated transport-verification ticket needs its own credit.
    let (_, finish_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    let _high_full = high_pool
        .charge(
            p.f.high.context.scope.group,
            crate::voice_v2::resources::GROUP_BYTES - high_pool.counts().0,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(
        open(
            &mut p.f.high,
            &finish_open,
            &plain,
            &p.f.clock,
            &mut p.f.rng
        )
        .outcome,
        CryptoOutcome::Opened
    );
    assert!(p.f.low.established(p.f.id, &p.f.clock));
    assert!(p.f.high.established(p.f.id, &p.f.clock));
    drop(low_full);
}

#[test]
fn protected_proof_has_three_global_inbox_identities_and_fourth_never_spends_more_capacity() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, plain) = seal(&mut p.f.low, &hello, &p.f.clock);
    let (_, hello_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &hello_open, &plain, &p.f.clock, &mut p.f.rng);
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &hello_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, mut plain) = seal(&mut p.f.high, &proof, &p.f.clock);
    plain[0] ^= 1;
    let mut retained = Vec::new();
    for lane in 0..3 {
        let mut body = object.body().clone();
        let Body::Encrypted { header, .. } = &mut body else {
            panic!("Proof");
        };
        header.sequence = lane + 1;
        let packet = SignedObject::issue(
            p.f.high.context.scope,
            &p.f.high.signer,
            [190 + lane as u8; 32],
            1000,
            16000,
            body,
        )
        .unwrap();
        let (receipt, action) = admit(&mut p.f.low, &p.low, packet.clone(), &p.f.clock);
        assert_eq!(receipt.candidate(), lane as u8);
        assert!(matches!(
            open(&mut p.f.low, &action, &plain, &p.f.clock, &mut p.f.rng).outcome,
            CryptoOutcome::Rejected(_)
        ));
        retained.push((packet, receipt));
    }
    let mut body = object.body().clone();
    let Body::Encrypted { header, .. } = &mut body else {
        panic!("Proof");
    };
    header.sequence = 4;
    let packet = SignedObject::issue(
        p.f.high.context.scope,
        &p.f.high.signer,
        [199; 32],
        1000,
        16000,
        body,
    )
    .unwrap();
    let before = p.f.pool.counts();
    assert_eq!(
        try_admit(&mut p.f.low, &p.low, packet, &p.f.clock).err(),
        Some(VoiceError::RateLimited)
    );
    assert_eq!(p.f.pool.counts(), before);
    for (object, receipt) in retained {
        assert!(Arc::ptr_eq(
            &receipt,
            &admit(&mut p.f.low, &p.low, object, &p.f.clock).0
        ));
    }
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .high_nonce
        .is_none());
}

#[test]
fn protected_stale_proof_admitted_after_new_start_rejects_without_spending_current_finish() {
    let mut p = fixture();
    let hello_a = start(&mut p, None);
    let a_id = message(&hello_a);
    let (object, plain) = seal(&mut p.f.low, &hello_a, &p.f.clock);
    let (_, open_a) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &open_a, &plain, &p.f.clock, &mut p.f.rng);
    let proof_a = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &open_a,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (a_object, a_plain) = seal(&mut p.f.high, &proof_a, &p.f.clock);
    let hello_b = start(&mut p, Some(a_id));
    let (object, plain) = seal(&mut p.f.low, &hello_b, &p.f.clock);
    let (_, open_b) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &open_b, &plain, &p.f.clock, &mut p.f.rng);
    let proof_b = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &open_b,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (b_object, b_plain) = seal(&mut p.f.high, &proof_b, &p.f.clock);
    // Arrival order B then A differs from the signed sequence order; inboxes remain distinct.
    let (b_receipt, b_open) = admit(&mut p.f.low, &p.low, b_object, &p.f.clock);
    let (a_receipt, a_open) = admit(&mut p.f.low, &p.low, a_object, &p.f.clock);
    assert_eq!((b_receipt.candidate(), a_receipt.candidate()), (0, 1));
    let rejected = open(&mut p.f.low, &a_open, &a_plain, &p.f.clock, &mut p.f.rng);
    assert_eq!(
        rejected.outcome,
        CryptoOutcome::Rejected(VoiceError::Unauthorized)
    );
    assert!(!rejected.retired);
    assert_eq!(
        open(&mut p.f.low, &b_open, &b_plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Opened
    );
    assert_eq!(
        respond(
            &mut p.f.low,
            &p.low,
            p.f.id,
            &a_open,
            &p.f.clock,
            &mut p.f.rng
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    let finish_b = respond(
        &mut p.f.low,
        &p.low,
        p.f.id,
        &b_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, plain) = seal(&mut p.f.low, &finish_b, &p.f.clock);
    let (_, finish_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    assert_eq!(
        open(
            &mut p.f.high,
            &finish_open,
            &plain,
            &p.f.clock,
            &mut p.f.rng
        )
        .outcome,
        CryptoOutcome::Opened
    );
}

#[test]
fn protected_malformed_hello_does_not_shift_the_valid_inputs_response_lane() {
    let mut p = fixture();
    let hello_a = start(&mut p, None);
    let a_id = message(&hello_a);
    let (object, mut plain) = seal(&mut p.f.low, &hello_a, &p.f.clock);
    let (a_receipt, a_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    plain[0] ^= 1;
    assert!(matches!(
        open(&mut p.f.high, &a_open, &plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Rejected(_)
    ));
    assert_eq!(p.f.high.join.confirmations.pairs[&p.f.id].starts, 0);
    let hello_b = start(&mut p, Some(a_id));
    let (object, plain) = seal(&mut p.f.low, &hello_b, &p.f.clock);
    let (b_receipt, b_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    assert_eq!((a_receipt.candidate(), b_receipt.candidate()), (0, 1));
    assert_eq!(
        open(&mut p.f.high, &b_open, &plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Opened
    );
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &b_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    assert_eq!(p.f.high.join.confirmations.pairs[&p.f.id].starts, 1);
    assert_eq!(
        respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &a_open,
            &p.f.clock,
            &mut p.f.rng
        )
        .err(),
        Some(VoiceError::Unauthorized)
    );
    let (object, plain) = seal(&mut p.f.high, &proof, &p.f.clock);
    let (_, low_open) = admit(&mut p.f.low, &p.low, object, &p.f.clock);
    assert_eq!(
        open(&mut p.f.low, &low_open, &plain, &p.f.clock, &mut p.f.rng).outcome,
        CryptoOutcome::Opened
    );
    respond(
        &mut p.f.low,
        &p.low,
        p.f.id,
        &low_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
}

#[test]
fn protected_response_final_capacity_refusal_publishes_nothing_and_exact_trigger_can_retry() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, plain) = seal(&mut p.f.low, &hello, &p.f.clock);
    let (_, trigger) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &trigger, &plain, &p.f.clock, &mut p.f.rng);
    let before = p._runtimes[1].resources().counts();
    let end = &mut p.f.high;
    let room = room(end);
    let refused = end.join.respond_protected_confirmation(
        ProtectedResponse {
            actor: &p.high,
            room: &room,
            handshake: p.f.id,
            trigger: &trigger,
        },
        &ca(&end.context, &end.lease),
        Arc::new(p.f.clock.clone()),
        &mut p.f.rng,
        |_, _| Err(VoiceError::Capacity),
    );
    assert_eq!(refused.err(), Some(VoiceError::Capacity));
    assert_eq!(p._runtimes[1].resources().counts(), before);
    assert!(end.join.confirmations.pairs[&p.f.id]
        .action
        .as_ref()
        .unwrap()
        .draft
        .is_none());
    let response = respond(end, &p.high, p.f.id, &trigger, &p.f.clock, &mut p.f.rng).unwrap();
    let input = response.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("Proof");
    };
    assert_eq!(
        flight.reservation.sequence, 1,
        "local refusal never published the direction sequence"
    );
}

#[test]
fn protected_response_final_call_revocation_cannot_publish_even_if_the_outer_observer_returns_success(
) {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, plain) = seal(&mut p.f.low, &hello, &p.f.clock);
    let (_, trigger) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &trigger, &plain, &p.f.clock, &mut p.f.rng);
    let end = &mut p.f.high;
    let room = room(end);
    let original = end.lease.clone();
    let refused = end.join.respond_protected_confirmation(
        ProtectedResponse {
            actor: &p.high,
            room: &room,
            handshake: p.f.id,
            trigger: &trigger,
        },
        &ca(&end.context, &end.lease),
        Arc::new(p.f.clock.clone()),
        &mut p.f.rng,
        |_, _| {
            original.revoke();
            Ok(())
        },
    );
    assert_eq!(refused.err(), Some(VoiceError::Unauthorized));
    assert!(end.join.confirmations.pairs[&p.f.id]
        .action
        .as_ref()
        .unwrap()
        .draft
        .is_none());
    assert!(respond(end, &p.high, p.f.id, &trigger, &p.f.clock, &mut p.f.rng).is_err());
}

#[test]
fn protected_old_response_recovers_but_cannot_run_and_old_unproduced_trigger_cannot_snapshot_new_state(
) {
    for produce_old in [false, true] {
        let mut p = fixture();
        let hello_a = start(&mut p, None);
        let a_id = message(&hello_a);
        let (object, plain) = seal(&mut p.f.low, &hello_a, &p.f.clock);
        let (_, open_a) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
        open(&mut p.f.high, &open_a, &plain, &p.f.clock, &mut p.f.rng);
        let old = produce_old.then(|| {
            respond(
                &mut p.f.high,
                &p.high,
                p.f.id,
                &open_a,
                &p.f.clock,
                &mut p.f.rng,
            )
            .unwrap()
        });
        let hello_b = start(&mut p, Some(a_id));
        let (object, plain) = seal(&mut p.f.low, &hello_b, &p.f.clock);
        let (_, open_b) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
        open(&mut p.f.high, &open_b, &plain, &p.f.clock, &mut p.f.rng);
        let before = p.f.rng.clone().next_u64();
        let recovered = respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &open_a,
            &p.f.clock,
            &mut p.f.rng,
        );
        if let Some(old) = old {
            assert!(Arc::ptr_eq(&old, &recovered.unwrap()));
            assert_eq!(
                old.admit_engine(old.engine_bytes()).err(),
                Some(VoiceError::Unauthorized)
            );
            assert!(
                old.input().is_err(),
                "proven unstarted superseded response retires outside the decrypt fence"
            );
        } else {
            assert_eq!(recovered.err(), Some(VoiceError::Unauthorized));
        }
        assert_eq!(p.f.rng.clone().next_u64(), before);
        respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &open_b,
            &p.f.clock,
            &mut p.f.rng,
        )
        .unwrap();
    }
}

#[test]
fn protected_followup_inbox_survives_attempt_replacement_but_old_open_never_progresses() {
    // Supersede before engine admission and after real engine admission. Both openings belong
    // to the same bounded inbox, while their plaintext echo remains tied to the older nonce.
    for engine_started in [false, true] {
        let mut p = fixture();
        let hello_a = start(&mut p, None);
        let a_id = message(&hello_a);
        let (object, plain) = seal(&mut p.f.low, &hello_a, &p.f.clock);
        let (_, hello_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
        open(&mut p.f.high, &hello_open, &plain, &p.f.clock, &mut p.f.rng);
        let proof = respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &hello_open,
            &p.f.clock,
            &mut p.f.rng,
        )
        .unwrap();
        let (object, plain) = seal(&mut p.f.high, &proof, &p.f.clock);
        let (_, proof_open) = admit(&mut p.f.low, &p.low, object, &p.f.clock);
        if engine_started {
            proof_open.admit_engine(proof_open.engine_bytes()).unwrap();
        }
        let hello_b = start(&mut p, Some(a_id));
        let b_id = message(&hello_b);
        let result = open(&mut p.f.low, &proof_open, &plain, &p.f.clock, &mut p.f.rng);
        assert_eq!(
            result.outcome,
            CryptoOutcome::Rejected(VoiceError::Unauthorized)
        );
        assert!(!result.retired);
        assert_eq!(
            p.f.low.join.confirmations.pairs[&p.f.id]
                .action
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .id,
            b_id
        );
        assert_eq!(
            respond(
                &mut p.f.low,
                &p.low,
                p.f.id,
                &proof_open,
                &p.f.clock,
                &mut p.f.rng
            )
            .err(),
            Some(VoiceError::Unauthorized)
        );
        // A terminal rejection is attached to the original inbox, not B's unrelated attempt.
        // C retires B, yet exact rejected history stays recoverable without retained input.
        start(&mut p, Some(b_id));
        assert!(!proof_open.terminal_status().receipt.unwrap().retired);
        assert_eq!(
            proof_open
                .recover_result(&plain, proof_open.output_tag(&plain).unwrap())
                .unwrap()
                .unwrap()
                .outcome,
            CryptoOutcome::Rejected(VoiceError::Unauthorized)
        );
        let mut changed = plain.clone();
        changed[0] ^= 1;
        assert_eq!(
            proof_open
                .recover_result(&changed, proof_open.output_tag(&changed).unwrap())
                .err(),
            Some(VoiceError::RetryMismatch)
        );
    }
}

#[test]
fn protected_old_finish_crossing_new_hello_rejects_without_establishing_the_new_attempt() {
    let mut p = fixture();
    let hello_a = start(&mut p, None);
    let a_id = message(&hello_a);
    let (object, plain) = seal(&mut p.f.low, &hello_a, &p.f.clock);
    let Body::Encrypted {
        header: mut old_header,
        ..
    } = object.body().clone()
    else {
        panic!("Hello header");
    };
    let (_, hello_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &hello_open, &plain, &p.f.clock, &mut p.f.rng);
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &hello_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, plain) = seal(&mut p.f.high, &proof, &p.f.clock);
    let (_, proof_open) = admit(&mut p.f.low, &p.low, object, &p.f.clock);
    open(&mut p.f.low, &proof_open, &plain, &p.f.clock, &mut p.f.rng);
    let finish = respond(
        &mut p.f.low,
        &p.low,
        p.f.id,
        &proof_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    // Model a delayed authenticated old Finish without signing/promoting the local endpoint;
    // the issuer possesses its pair key. The receiving state machine must still reject it.
    let input = finish.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("finish");
    };
    let old_plain = flight.plaintext.clone();
    old_header.kind = EncryptedKind::ConfirmFinish;
    old_header.sequence = flight.reservation.sequence;
    let old = SignedObject::issue(
        p.f.low.context.scope,
        &p.f.low.signer,
        [233; 32],
        1000,
        16000,
        Body::Encrypted {
            header: old_header,
            ciphertext: vec![22; old_plain.len() + 16],
        },
    )
    .unwrap();
    let (_, old_open) = admit(&mut p.f.high, &p.high, old, &p.f.clock);
    let hello_b = start(&mut p, Some(a_id));
    let (object, plain) = seal(&mut p.f.low, &hello_b, &p.f.clock);
    let (_, new_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &new_open, &plain, &p.f.clock, &mut p.f.rng);
    let result = open(
        &mut p.f.high,
        &old_open,
        &old_plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    assert_eq!(
        result.outcome,
        CryptoOutcome::Rejected(VoiceError::Unauthorized)
    );
    assert!(!p.f.high.established(p.f.id, &p.f.clock));
    respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &new_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
}

#[test]
fn protected_response_final_origin_clock_observes_each_original_owner_revocation_before_publication(
) {
    fn ready(p: &mut ProtectedFixture) -> Arc<CryptoAction> {
        let hello = start(p, None);
        let (object, plain) = seal(&mut p.f.low, &hello, &p.f.clock);
        let (_, trigger) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
        open(&mut p.f.high, &trigger, &plain, &p.f.clock, &mut p.f.rng);
        trigger
    }
    // Measure the actual final origin Clock observation on a control run. This avoids coupling
    // the regression to the number of earlier preparation observations or adding a runtime hook.
    let mut control = fixture();
    let trigger = ready(&mut control);
    let before = control
        .origin_clock
        .reads
        .load(std::sync::atomic::Ordering::Acquire);
    respond(
        &mut control.f.high,
        &control.high,
        control.f.id,
        &trigger,
        &control.f.clock,
        &mut control.f.rng,
    )
    .unwrap();
    let observations = control
        .origin_clock
        .reads
        .load(std::sync::atomic::Ordering::Acquire)
        - before;
    assert!(observations > 0);
    for gate in 0..4 {
        let mut p = fixture();
        let trigger = ready(&mut p);
        let pair = &p.f.high.join.confirmations.pairs[&p.f.id];
        let original = match gate {
            0 => p.f.high.lease.clone(),
            1 => pair.action.as_ref().unwrap().live.0.clone(),
            2 => pair.attempt.0.clone(),
            _ => pair.chain.as_ref().unwrap().lease(),
        };
        let read = p
            .origin_clock
            .reads
            .load(std::sync::atomic::Ordering::Acquire)
            + observations;
        *p.origin_clock.revoke.lock().unwrap() = Some((read, original));
        assert_eq!(
            respond(
                &mut p.f.high,
                &p.high,
                p.f.id,
                &trigger,
                &p.f.clock,
                &mut p.f.rng
            )
            .err(),
            Some(VoiceError::Unauthorized)
        );
        assert!(
            p.origin_clock.revoke.lock().unwrap().is_none(),
            "actual final Clock callback fired"
        );
        assert!(p.f.high.join.confirmations.pairs[&p.f.id]
            .action
            .as_ref()
            .unwrap()
            .draft
            .is_none());
    }
}

#[test]
fn protected_sealed_old_response_keeps_history_but_loses_actual_queued_source_authority() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let predecessor = message(&hello);
    let (object, plain) = seal(&mut p.f.low, &hello, &p.f.clock);
    let (_, trigger) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(&mut p.f.high, &trigger, &plain, &p.f.clock, &mut p.f.rng);
    let proof = respond(
        &mut p.f.high,
        &p.high,
        p.f.id,
        &trigger,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (signed, _) = seal(&mut p.f.high, &proof, &p.f.clock);
    let source =
        p.f.high
            .join
            .source_packet_activity(
                &signed,
                &ca(&p.f.high.context, &p.f.high.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    assert!(source.check(&signed).is_ok());
    let hello_b = start(&mut p, Some(predecessor));
    let (object, plain) = seal(&mut p.f.low, &hello_b, &p.f.clock);
    let (_, new_trigger) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    open(
        &mut p.f.high,
        &new_trigger,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    assert!(source.check(&signed).is_err());
    assert!(Arc::ptr_eq(
        &proof,
        &respond(
            &mut p.f.high,
            &p.high,
            p.f.id,
            &trigger,
            &p.f.clock,
            &mut p.f.rng
        )
        .unwrap()
    ));
    assert!(proof.terminal_status().receipt.unwrap().retired);
    assert!(proof.input().is_err());
    assert_eq!(
        proof.admit_engine(proof.engine_bytes()).err(),
        Some(VoiceError::Unauthorized)
    );
}
