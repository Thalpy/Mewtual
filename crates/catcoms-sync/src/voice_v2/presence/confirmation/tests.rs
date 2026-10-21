//! Native state tests mock only the trusted browser AEAD return. Original signatures, immutable
//! headers, replay/token ownership and confirmation transitions are real; WebCrypto has separate
//! real-engine tests. A fake ciphertext here makes no confidentiality/physical-media claim.
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
mod protected;
mod protected_start;

fn members(_: DeviceId) -> bool {
    true
}
fn ca<'a>(context: &'a Context, lease: &'a CallLease) -> Authority<'a> {
    Authority {
        context,
        lease,
        member: &members,
    }
}
fn wa<'a>(context: &'a Context, lease: &'a WatchLease) -> WatchAuthority<'a> {
    WatchAuthority {
        context,
        lease,
        member: &members,
    }
}
fn key(seed: u8) -> JoinKey {
    let point = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    JoinKey::new([seed; 32], point.try_into().unwrap()).unwrap()
}
struct End {
    signer: DeviceKeypair,
    context: Context,
    lease: CallLease,
    join: LocalJoin,
    watcher: Watcher,
    watch_lease: WatchLease,
    limits: OriginLimits,
}
impl End {
    fn new(seed: u8, pool: Arc<Resources>, rng: &mut ChaCha20Rng) -> Self {
        let signer = DeviceKeypair::from_seed(&[seed; 32]);
        let context = Context {
            scope: Scope::for_group(b"exact confirmation group", Some([7; 32]), 44),
            local: signer.device_id(),
            now: 1000,
            epoch: 3,
            indirect: true,
        };
        let lease = CallLease::default();
        let watch_lease = WatchLease::default();
        let join = LocalJoin::new(&context, key(seed), lease.clone(), pool.clone()).unwrap();
        let watcher = Watcher::new(&context, watch_lease.clone(), pool.clone(), rng).unwrap();
        let limits = OriginLimits::new(context.scope.group, pool);
        Self {
            signer,
            context,
            lease,
            join,
            watcher,
            watch_lease,
            limits,
        }
    }
    fn announce(&mut self, clock: &ManualClock, rng: &mut ChaCha20Rng) -> SignedObject {
        self.join
            .announce(&self.signer, &ca(&self.context, &self.lease), clock, rng)
            .unwrap()
    }
    fn challenge(
        &mut self,
        ann: &SignedObject,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> SignedObject {
        self.watcher
            .challenge(
                ann,
                &self.signer,
                &wa(&self.context, &self.watch_lease),
                &mut self.limits,
                clock,
                rng,
            )
            .unwrap()
    }
    fn answer(
        &mut self,
        challenge: &SignedObject,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> SignedObject {
        self.join
            .answer(
                challenge,
                &self.signer,
                &ca(&self.context, &self.lease),
                clock,
                rng,
            )
            .unwrap()
    }
    fn begin(&mut self, proof: &SignedObject, clock: &ManualClock) -> PairSetup {
        let fresh = self
            .watcher
            .prove(proof, &wa(&self.context, &self.watch_lease), clock)
            .unwrap();
        self.join
            .begin_pair(
                &fresh,
                &ca(&self.context, &self.lease),
                &mut self.limits,
                clock,
            )
            .unwrap()
    }
    fn start(
        &mut self,
        id: Nonce,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> Result<(), VoiceError> {
        self.join
            .start_confirmation(id, &ca(&self.context, &self.lease), clock, rng)
    }
    fn prepare(
        &mut self,
        id: Nonce,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> Result<Option<ConfirmationOutput>, VoiceError> {
        self.join
            .prepare_confirmation(id, &ca(&self.context, &self.lease), clock, rng)
    }
    fn flight(&mut self, id: Nonce, clock: &ManualClock, rng: &mut ChaCha20Rng) -> CryptoFlight {
        let Some(ConfirmationOutput::Encrypt(flight)) = self.prepare(id, clock, rng).unwrap()
        else {
            panic!("expected exact crypto reservation")
        };
        flight
    }
    fn sign(
        &mut self,
        flight: &CryptoFlight,
        clock: &ManualClock,
    ) -> Result<SignedObject, VoiceError> {
        self.join.submit_confirmation(
            FlightSubmission {
                handshake: flight.handshake,
                action: flight.action,
                reservation: flight.reservation.id,
                ciphertext: &vec![22; flight.plaintext.len() + 16],
            },
            &self.signer,
            &ca(&self.context, &self.lease),
            clock,
        )
    }
    fn receive(
        &mut self,
        object: SignedObject,
        clock: &ManualClock,
    ) -> Result<ReceiveProgress, VoiceError> {
        self.join
            .receive_confirmation(object, &ca(&self.context, &self.lease), clock)
    }
    fn commit(
        &mut self,
        object: SignedObject,
        plain: &[u8],
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> Result<(), VoiceError> {
        let ReceiveProgress::Decrypt(incoming) = self.receive(object, clock)? else {
            return Err(VoiceError::Replay);
        };
        self.join.commit_confirmation(
            OpenedFlight {
                token: incoming.token,
                aad: incoming.aad(),
                sequence: incoming.header.sequence,
                ciphertext: incoming.ciphertext(),
                plaintext: plain,
            },
            &ca(&self.context, &self.lease),
            clock,
            rng,
        )
    }
    fn established(&mut self, id: Nonce, clock: &ManualClock) -> bool {
        self.join
            .pair_established(id, &ca(&self.context, &self.lease), clock)
            .unwrap()
    }
}
struct Fixture {
    low: End,
    high: End,
    id: Nonce,
    clock: ManualClock,
    rng: ChaCha20Rng,
    pool: Arc<Resources>,
}
fn admit_pair(
    left: &mut End,
    right: &mut End,
    clock: &ManualClock,
    rng: &mut ChaCha20Rng,
) -> Nonce {
    let ann_left = left.announce(clock, rng);
    let ann_right = right.announce(clock, rng);
    let challenge_left = left.challenge(&ann_right, clock, rng);
    let proof_right = right.answer(&challenge_left, clock, rng);
    let challenge_right = right.challenge(&ann_left, clock, rng);
    let proof_left = left.answer(&challenge_right, clock, rng);
    let left_setup = left.begin(&proof_right, clock);
    let right_setup = right.begin(&proof_left, clock);
    assert_eq!(left_setup.handshake, right_setup.handshake);
    assert_eq!(left_setup.transcript, right_setup.transcript);
    assert_ne!(left_setup.local_is_low, right_setup.local_is_low);
    left_setup.handshake
}
fn fixture() -> Fixture {
    fixture_with_resources(ManualClock::new(1000), Arc::new(Resources::default()))
}
fn fixture_with_resources(clock: ManualClock, pool: Arc<Resources>) -> Fixture {
    let mut rng = ChaCha20Rng::from_seed([99; 32]);
    let a = End::new(1, pool.clone(), &mut rng);
    let b = End::new(2, pool.clone(), &mut rng);
    let (mut low, mut high) = if a.context.local < b.context.local {
        (a, b)
    } else {
        (b, a)
    };
    let id = admit_pair(&mut low, &mut high, &clock, &mut rng);
    Fixture {
        low,
        high,
        id,
        clock,
        rng,
        pool,
    }
}
fn hello(f: &mut Fixture) -> (CryptoFlight, SignedObject) {
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&flight, &f.clock).unwrap();
    (flight, signed)
}

#[test]
fn native_pair_nonce_tags_fit_existing_record_with_maximum_derivation_transcript() {
    // PairSetup derivation input is capped at4096; canonical native pair transcripts are
    // smaller. Include the actual enlarged Pair layout without raising its original allowance.
    assert!(std::mem::size_of::<Pair>() + 4096 <= PAIR_BYTES);
    let f = fixture();
    for end in [&f.low, &f.high] {
        assert!(end.join.confirmations.pairs[&f.id].transcript.len() <= 4096);
    }
}

#[test]
fn prepared_chain_is_same_pair_idempotent_and_close_cannot_reset_it() {
    let mut f = fixture();
    let before = f.pool.counts();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    let after = f.pool.counts();
    assert!(after.0 > before.0);
    let again = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    assert!(Arc::ptr_eq(&chain, &again));
    assert_eq!(f.pool.counts(), after);
    assert_eq!(f.low.join.confirmations.pairs[&f.id].starts, 0);
    chain.close();
    assert_eq!(
        f.pool.counts(),
        (before.0 + 2048, before.1),
        "closed fixed metadata stays charged"
    );
    assert!(f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .is_err());
    assert!(f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock),
            ByteBudget::Ordinary
        )
        .is_err());
}

#[test]
fn protected_mode_survives_capacity_refusal_and_cannot_fall_back_to_ordinary_handshake() {
    for high_side in [false, true] {
        let mut f = fixture();
        // The other endpoint may already have sent Hello; this endpoint is still pristine.
        let incoming = high_side.then(|| hello(&mut f).1);
        let pressure = f
            .pool
            .charge(
                f.low.context.scope.group,
                256 * 1024 - f.pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
        let end = if high_side { &mut f.high } else { &mut f.low };
        let before = f.pool.counts();
        assert_eq!(
            end.join
                .prepare_crypto_chain(f.id, &ca(&end.context, &end.lease), &f.clock)
                .err(),
            Some(VoiceError::Capacity)
        );
        let pair = &end.join.confirmations.pairs[&f.id];
        assert_eq!(pair.protected_epoch, Some(end.context.epoch));
        assert!(pair.chain.is_none());
        assert_eq!(
            f.pool.counts(),
            before,
            "failed capacity acquisition must return unused claims"
        );
        assert_eq!(
            end.join
                .derivation_action(
                    f.id,
                    &ca(&end.context, &end.lease),
                    Arc::new(f.clock.clone()),
                    ByteBudget::Ordinary
                )
                .err(),
            Some(VoiceError::Unauthorized)
        );
        assert_eq!(
            end.join
                .prepare_confirmation(f.id, &ca(&end.context, &end.lease), &f.clock, &mut f.rng)
                .err(),
            Some(VoiceError::Unauthorized)
        );
        if let Some(incoming) = incoming {
            assert_eq!(
                end.receive(incoming, &f.clock).err(),
                Some(VoiceError::Unauthorized)
            );
        } else {
            assert_eq!(
                end.start(f.id, &f.clock, &mut f.rng),
                Err(VoiceError::Unauthorized)
            );
        }
        drop(pressure);
        let chain = end
            .join
            .prepare_crypto_chain(f.id, &ca(&end.context, &end.lease), &f.clock)
            .unwrap();
        assert!(Arc::ptr_eq(
            &chain,
            end.join.confirmations.pairs[&f.id].chain.as_ref().unwrap()
        ));
        assert_eq!(end.join.confirmations.pairs[&f.id].starts, 0);
    }
}

#[test]
fn protected_selection_refuses_an_existing_ordinary_hello_or_decrypt_capability() {
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let before = f.pool.counts();
    assert_eq!(
        f.low
            .join
            .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.pool.counts(), before);
    assert!(f.low.join.confirmations.pairs[&f.id]
        .protected_epoch
        .is_none());
    // Refusing migration does not damage the already selected ordinary handshake.
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&flight, &f.clock).unwrap();
    let ReceiveProgress::Decrypt(event) = f.high.receive(signed, &f.clock).unwrap() else {
        panic!("decrypt");
    };
    let action = f
        .high
        .join
        .confirmation_decryption_action(
            *event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert_eq!(
        f.high.join.confirmations.pairs[&f.id].starts, 0,
        "a queued Hello has not changed decoded confirmation state"
    );
    let before = f.pool.counts();
    assert_eq!(
        f.high
            .join
            .prepare_crypto_chain(f.id, &ca(&f.high.context, &f.high.lease), &f.clock)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.pool.counts(), before);
    assert!(f.high.join.confirmations.pairs[&f.id]
        .protected_epoch
        .is_none());
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = f
        .high
        .join
        .commit_confirmation_result(
            &action,
            &flight.plaintext,
            action.output_tag(&flight.plaintext).unwrap(),
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Opened);
    action.settle_engine();
}

#[test]
fn prepared_chain_protects_real_packet_capacity_and_close_returns_only_unused_claims() {
    let mut f = fixture();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    let mut reserve = || {
        f.low.join.ledger.reserve(
            f.low.signer.verifying_key().to_bytes(),
            ReserveRequest {
                handshake: f.id,
                kind: EncryptedKind::ConfirmHello,
                offer: None,
                ciphertext_bytes: 32,
            },
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
            &mut f.rng,
        )
    };
    for _ in 0..26 {
        reserve().unwrap();
    }
    assert_eq!(reserve().err(), Some(VoiceError::Capacity));
    chain.close();
    for _ in 0..6 {
        reserve().unwrap();
    }
    assert_eq!(reserve().err(), Some(VoiceError::Capacity));
    // Closing a low-side workflow returns its six unused output claims, not any ordinary rows
    // already published in this exact ledger. The original total limit remains unchanged.
}

#[test]
fn prepared_chain_derive_uses_protected_capacity_and_refuses_replacement_or_other_join() {
    let mut f = fixture();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    assert!(f
        .high
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone())
        )
        .is_err());
    // Fill ordinary bytes and all unreserved action slots. The native factory must borrow the
    // plan's fixed row, not take another ordinary slot/byte before reaching its reserve.
    let slots = f.low.join.ledger.action_slots();
    let ordinary_slots = slots.charge(20).unwrap();
    let pressure = f
        .pool
        .charge(
            f.low.context.scope.group,
            256 * 1024 - f.pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let action = f
        .low
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    let again = f
        .low
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&action, &again));
    let used = f.pool.counts();
    action.admit_engine(action.engine_bytes()).unwrap();
    assert_eq!(
        f.pool.counts(),
        used,
        "engine uses its prepaid lane and bytes"
    );
    assert!(f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary
        )
        .is_err());
    f.low.context.epoch += 1;
    assert!(f
        .low
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone())
        )
        .is_err());
    assert!(f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .is_err());
    assert!(!action.is_pending());
    action.settle_engine();
    drop((pressure, ordinary_slots));
}

#[test]
fn ordinary_derivation_cannot_be_adopted_and_chain_retirement_does_not_reenter_result_fence() {
    let mut f = fixture();
    let action = f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert!(f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .is_err());
    action.retire_if_unstarted();

    let mut f = fixture();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    let action = f
        .low
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let local = f.low.context.local;
    let not_member = |candidate: DeviceId| candidate == local;
    let stale = Authority {
        context: &f.low.context,
        lease: &f.low.lease,
        member: &not_member,
    };
    // This enters pair_current while Engine is held. It must only revoke the pair pin;
    // housekeeping, outside Engine, subsequently closes/retire unused preparation.
    assert!(f
        .low
        .join
        .commit_derivation_result(&action, action.output_tag(&[]).unwrap(), &stale, &f.clock)
        .is_err());
    f.pool.collect_workflows(f.clock.monotonic_ms());
    assert!(action.with_pending(|| Ok(())).is_err());
    action.settle_engine();
    assert!(chain.prepare_derive().is_err());
}

#[test]
fn prepared_chain_first_derive_result_rejects_epoch_change_without_another_factory_call() {
    let mut f = fixture();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    let action = f
        .low
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    f.low.context.epoch += 1;
    assert!(f
        .low
        .join
        .commit_derivation_result(
            &action,
            action.output_tag(&[]).unwrap(),
            &ca(&f.low.context, &f.low.lease),
            &f.clock
        )
        .is_err());
    assert!(action.terminal_status().receipt.is_none());
    assert!(action
        .recover_result(&[], action.output_tag(&[]).unwrap())
        .unwrap()
        .is_none());
    // Neither a failed first effect nor current epoch data can replenish the reserved row.
    assert!(chain.prepare_derive().is_err());
    action.settle_engine();
    chain.close();

    let mut f = fixture();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    let action = f
        .low
        .join
        .chain_derivation_action(
            f.id,
            &chain,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = f
        .low
        .join
        .commit_derivation_result(
            &action,
            action.output_tag(&[]).unwrap(),
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    f.low.context.epoch += 1;
    let recovered = f
        .low
        .join
        .commit_derivation_result(
            &action,
            receipt.output_tag,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    assert!(
        recovered.same_transition(&receipt),
        "current epoch cannot erase exact past result metadata"
    );
}

#[test]
fn native_action_repeated_registration_and_pull_share_one_engine_fence() {
    let mut f = fixture();
    let action = f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    let again = f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&action, &again));
    assert_eq!(action.operation(), crypto_actions::CryptoOperation::Derive);
    assert_eq!(action.version(), 1);
    assert!(action.belongs_to(f.low.context.scope, &f.low.lease));
    assert!(!action.belongs_to(f.low.context.scope, &f.high.lease));
    assert!(action.uses_resources(&f.pool));
    assert!(!action.uses_resources(&Arc::new(Resources::default())));
    let first = f
        .low
        .join
        .pull_crypto_actions(&ca(&f.low.context, &f.low.lease))
        .unwrap();
    let second = f
        .low
        .join
        .pull_crypto_actions(&ca(&f.low.context, &f.low.lease))
        .unwrap();
    assert!(Arc::ptr_eq(&first[0], &second[0]));
    let before = f.pool.counts();
    assert!(action.admit_engine(action.engine_bytes() - 1).is_err());
    assert_eq!(f.pool.counts(), before);
    let permit = action.admit_engine(action.engine_bytes()).unwrap();
    assert_eq!(action.admit_engine(action.engine_bytes()).unwrap(), permit);
    assert_eq!(f.pool.counts(), (before.0 + action.engine_bytes(), 1));
    let reader = action.input().unwrap();
    let CryptoInput::Derive(setup) = reader.as_input() else {
        panic!("derive input")
    };
    let payload_bytes = crypto_actions::derive_bytes(setup.transcript.len())
        .unwrap()
        .0;
    action.settle_engine();
    action.settle_engine();
    assert_eq!(f.pool.counts(), before);
    assert!(action.input().is_err());
    drop(reader);
    assert_eq!(f.pool.counts(), (before.0 - payload_bytes, 0));
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert!(!action.is_pending());
    assert!(f
        .low
        .join
        .pull_crypto_actions(&ca(&f.low.context, &f.low.lease))
        .unwrap()
        .is_empty());
}

#[test]
fn native_action_settle_before_admit_refuses_delayed_admission_and_first_sign() {
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    let action = f
        .low
        .join
        .confirmation_encryption_action(
            flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert_eq!(
        action.operation(),
        crypto_actions::CryptoOperation::EncryptConfirmation
    );
    let reader = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = reader.as_input() else {
        panic!("seal input")
    };
    let payload_bytes = flight.reservation.aad.len() + flight.plaintext.len() + 16;
    let before = f.pool.counts();
    action.settle_engine();
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert!(f
        .low
        .join
        .submit_confirmation_action(
            &action,
            &ciphertext,
            &f.low.signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock
        )
        .is_err());
    assert_eq!(f.pool.counts(), before);
    assert!(action.input().is_err());
    drop(reader);
    assert_eq!(f.pool.counts(), (before.0 - payload_bytes, 0));
    assert!(matches!(
        f.low.prepare(f.id, &f.clock, &mut f.rng).unwrap(),
        Some(ConfirmationOutput::Encrypt(_))
    ));
}

#[test]
fn native_action_actual_sign_and_closed_plaintext_use_the_registered_fence() {
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let plain = flight.plaintext.clone();
    let ciphertext = vec![22; plain.len() + 16];
    let seal = f
        .low
        .join
        .confirmation_encryption_action(
            flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert!(f
        .low
        .join
        .submit_confirmation_action(
            &seal,
            &ciphertext,
            &f.low.signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock
        )
        .is_err());
    seal.admit_engine(seal.engine_bytes()).unwrap();
    let signed = f
        .low
        .join
        .submit_confirmation_action(
            &seal,
            &ciphertext,
            &f.low.signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    assert!(
        !seal.is_pending(),
        "signing consumes pending encryption authority"
    );
    assert_eq!(
        f.pool.counts().1,
        1,
        "signing is not actual engine/RPC settlement"
    );
    let hash = signed.hash();
    let ReceiveProgress::Decrypt(event) = f.high.receive(signed, &f.clock).unwrap() else {
        panic!("decrypt");
    };
    let open = f
        .high
        .join
        .confirmation_decryption_action(
            *event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert_eq!(open.input_tag(), hash);
    open.admit_engine(open.engine_bytes()).unwrap();
    f.high
        .join
        .commit_confirmation_action(
            &open,
            &plain,
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    assert!(
        !open.is_pending(),
        "consumed native token cannot mint more engine work"
    );
    assert_eq!(f.pool.counts().1, 2);
    assert!(matches!(
        f.high.prepare(f.id, &f.clock, &mut f.rng).unwrap(),
        Some(ConfirmationOutput::Encrypt(_))
    ));
    seal.settle_engine();
    open.settle_engine();
    assert_eq!(f.pool.counts().1, 0);
}

#[test]
fn native_action_capacity_refusal_does_not_consume_identity_or_engine_eligibility() {
    let mut f = fixture();
    let bytes = 256 * 1024 - f.pool.counts().0;
    let pressure = f
        .pool
        .charge(f.low.context.scope.group, bytes, ResourceKind::Retained)
        .unwrap();
    assert!(f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary
        )
        .is_err());
    assert_eq!(f.pool.counts().0, 256 * 1024);
    drop(pressure);
    let action = f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    assert_eq!(
        action.serial(),
        1,
        "failed storage admission publishes/burns no action"
    );
    let engines: Vec<_> = (0..4)
        .map(|_| {
            f.pool
                .charge(f.low.context.scope.group, 0, ResourceKind::Crypto)
                .unwrap()
        })
        .collect();
    let before = f.pool.counts();
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert_eq!(f.pool.counts(), before);
    assert!(action.is_pending());
    drop(engines);
    assert!(action.admit_engine(action.engine_bytes()).is_ok());
    action.settle_engine();
}

#[test]
fn native_action_lost_accepted_open_reply_recovers_after_settlement() {
    lost_open_reply(false);
}
#[test]
fn native_action_lost_rejected_open_reply_recovers_after_settlement() {
    lost_open_reply(true);
}
fn lost_open_reply(malformed: bool) {
    let mut f = fixture();
    let (flight, signed) = hello(&mut f);
    let mut plaintext = flight.plaintext;
    if malformed {
        plaintext[0] ^= 1;
    }
    let ReceiveProgress::Decrypt(event) = f.high.receive(signed, &f.clock).unwrap() else {
        panic!("decrypt");
    };
    let action = f
        .high
        .join
        .confirmation_decryption_action(
            *event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    let tag = action.output_tag(&plaintext).unwrap();
    let before = f.pool.counts();
    assert_eq!(action.recover_result(&plaintext, tag).unwrap(), None);
    assert_eq!(
        f.pool.counts(),
        before,
        "a history miss neither executes nor admits an engine"
    );
    action.admit_engine(action.engine_bytes()).unwrap();
    let expected = f.high.join.commit_confirmation_action(
        &action,
        &plaintext,
        &ca(&f.high.context, &f.high.lease),
        &f.clock,
        &mut f.rng,
    );
    assert_eq!(expected.is_err(), malformed);
    assert_eq!(
        f.pool.counts().1,
        1,
        "terminal result does not settle the actual engine"
    );
    let receipt = f
        .high
        .join
        .commit_confirmation_result(
            &action,
            &plaintext,
            tag,
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    assert!(
        !receipt.retired,
        "accepted Hello binds its POST-transition attempt"
    );
    let mut changed = plaintext.clone();
    *changed.last_mut().unwrap() ^= 1;
    for claimed in [tag, action.output_tag(&changed).unwrap()] {
        assert_eq!(
            f.high.join.commit_confirmation_result(
                &action,
                &changed,
                claimed,
                &ca(&f.high.context, &f.high.lease),
                &f.clock,
                &mut f.rng
            ),
            Err(VoiceError::RetryMismatch)
        );
    }
    let pair = &f.high.join.confirmations.pairs[&f.id];
    let unchanged = (
        pair.starts,
        pair.action_serial,
        pair.low_nonce,
        pair.high_nonce,
    );
    action.settle_engine();
    let held = f.pool.counts();
    let recovered = f.high.join.commit_confirmation_action(
        &action,
        &plaintext,
        &ca(&f.high.context, &f.high.lease),
        &f.clock,
        &mut f.rng,
    );
    assert_eq!(recovered, expected, "exact native outcome must survive consumed token and lost result reply; malformed={malformed}");
    let pair = &f.high.join.confirmations.pairs[&f.id];
    assert_eq!(
        (
            pair.starts,
            pair.action_serial,
            pair.low_nonce,
            pair.high_nonce
        ),
        unchanged
    );
    assert_eq!(
        f.pool.counts(),
        held,
        "historical lookup neither runs another engine nor allocates a second result record"
    );
    assert!(!action.is_pending());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    f.high.lease.revoke();
    let retired = f
        .high
        .join
        .commit_confirmation_result(
            &action,
            &plaintext,
            tag,
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    assert!(retired.retired);
    assert_eq!(retired.outcome, receipt.outcome);
    assert_eq!(
        f.high.join.commit_confirmation_action(
            &action,
            &plaintext,
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng
        ),
        Err(VoiceError::Unauthorized)
    );
    drop(f.high.join);
    // The registered native Arc can survive actual LocalJoin/Root pruning. Its historical
    // success OR closed rejection remains exact metadata and cannot recreate first authority.
    let held = f.pool.counts();
    let recovered = action.recover_result(&plaintext, tag).unwrap().unwrap();
    assert!(recovered.retired);
    assert_eq!(recovered.outcome, receipt.outcome);
    // The browser may already have cleared its quarantined output at End. Status recovery
    // needs neither that output nor a surviving participant, and cannot validate a commit retry.
    let status = action.terminal_status();
    assert!(status.settled);
    assert_eq!(status.receipt.as_ref(), Some(&recovered));
    assert!(receipt.same_transition(status.receipt.as_ref().unwrap()));
    assert_ne!(receipt.retired, recovered.retired);
    assert_eq!(
        f.pool.counts(),
        held,
        "history/status lookup allocates no new record"
    );
    let input = action.input().unwrap();
    action.acknowledge_result(&recovered).unwrap();
    assert!(action.input().is_err());
    assert_eq!(
        f.pool.counts(),
        held,
        "borrowed input still owns all original ciphertext and action payload custody"
    );
    assert_eq!(
        action.recover_result(&plaintext, tag).unwrap(),
        Some(recovered.clone())
    );
    drop(input);
    assert!(f.pool.counts().0 < held.0);
    let compacted = f.pool.counts();
    for claimed in [tag, action.output_tag(&changed).unwrap()] {
        assert_eq!(
            action.recover_result(&changed, claimed),
            Err(VoiceError::RetryMismatch)
        );
    }
    assert_eq!(f.pool.counts(), compacted);
    assert!(action.admit_engine(action.engine_bytes()).is_err());
}

#[test]
fn native_action_lost_seal_reply_recovers_after_construction_deadline_without_resigning() {
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let ciphertext = vec![22; flight.plaintext.len() + 16];
    let action = f
        .low
        .join
        .confirmation_encryption_action(
            flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    struct CountingSigner<'a> {
        signer: &'a DeviceKeypair,
        calls: std::cell::Cell<usize>,
    }
    impl VoiceSigner for CountingSigner<'_> {
        fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
            self.signer.voice_public_key()
        }
        fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError> {
            self.calls.set(self.calls.get() + 1);
            self.signer.sign_voice(bytes)
        }
    }
    let signer = CountingSigner {
        signer: &f.low.signer,
        calls: std::cell::Cell::new(0),
    };
    f.clock.advance_ms(4_900);
    let accepted = f
        .low
        .join
        .submit_confirmation_action(
            &action,
            &ciphertext,
            &signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    action.settle_engine();
    f.clock.advance_ms(200);
    let recovered = f.low.join.submit_confirmation_action(
        &action,
        &ciphertext,
        &signer,
        &ca(&f.low.context, &f.low.lease),
        &f.clock,
    );
    assert_eq!(
        recovered,
        Ok(accepted),
        "historical success does not require the consumed five-second encryption grant"
    );
    let tag = action.output_tag(&ciphertext).unwrap();
    let receipt = action.recover_result(&ciphertext, tag).unwrap().unwrap();
    action.acknowledge_result(&receipt).unwrap();
    assert!(action.input().is_err());
    assert_eq!(
        f.low
            .join
            .submit_confirmation_result(
                &action,
                &ciphertext,
                tag,
                &signer,
                &ca(&f.low.context, &f.low.lease),
                &f.clock,
            )
            .unwrap(),
        receipt
    );
    assert_eq!(
        f.low.join.submit_confirmation_action(
            &action,
            &ciphertext,
            &signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        ),
        Err(VoiceError::Unauthorized),
        "acknowledged history cannot reconstruct fresh send data"
    );
    assert_eq!(signer.calls.get(), 1, "exact lookup does not sign again");
}

#[test]
fn native_derivation_history_uses_original_clock_and_ignores_nonce_attempt_replacement() {
    let mut f = fixture();
    let action = f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let tag = action.output_tag(&[]).unwrap();
    let ready = f
        .low
        .join
        .commit_derivation_result(&action, tag, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    assert_eq!(ready.outcome, CryptoOutcome::Derived);
    assert!(!ready.retired);
    action.settle_engine();
    action.acknowledge_result(&ready).unwrap();
    assert!(action.input().is_err());
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    assert_eq!(
        f.low
            .join
            .commit_derivation_result(&action, tag, &ca(&f.low.context, &f.low.lease), &f.clock)
            .unwrap(),
        ready
    );
    establish(&mut f);
    f.clock.advance_ms(16_000);
    let retry_clock = ManualClock::new(1_000);
    let expired = f
        .low
        .join
        .commit_derivation_result(
            &action,
            tag,
            &ca(&f.low.context, &f.low.lease),
            &retry_clock,
        )
        .unwrap();
    assert!(
        expired.retired,
        "cached lookup cannot choose a fresh clock or reset the original pair deadline"
    );
    assert_eq!(expired.outcome, ready.outcome);
}

#[test]
fn native_closed_open_prepares_serial_capacity_before_token_or_attempt_mutation() {
    let mut f = fixture();
    let (flight, signed) = hello(&mut f);
    let ReceiveProgress::Decrypt(event) = f.high.receive(signed, &f.clock).unwrap() else {
        panic!("decrypt")
    };
    let action = f
        .high
        .join
        .confirmation_decryption_action(
            *event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Ordinary,
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    f.high
        .join
        .confirmations
        .pairs
        .get_mut(&f.id)
        .unwrap()
        .action_serial = u64::MAX;
    let original_attempt = f.high.join.confirmations.pairs[&f.id].attempt.0.clone();
    let result = f.high.join.commit_confirmation_action(
        &action,
        &flight.plaintext,
        &ca(&f.high.context, &f.high.lease),
        &f.clock,
        &mut f.rng,
    );
    assert_eq!(result, Err(VoiceError::Capacity));
    let pair = &f.high.join.confirmations.pairs[&f.id];
    assert!(pair.attempt.0.is_same(&original_attempt));
    assert!(original_attempt.is_live());
    assert_eq!(pair.starts, 0);
    assert_eq!(pair.low_nonce, None);
    assert!(pair.action.is_none());
    assert!(
        action.is_pending(),
        "incoming token remains unconsumed on local preflight refusal"
    );
}

#[test]
fn native_action_expiry_end_and_root_drop_keep_admitted_engine_custody() {
    for transition in 0..3 {
        let mut f = fixture();
        let action = f
            .low
            .join
            .derivation_action(
                f.id,
                &ca(&f.low.context, &f.low.lease),
                Arc::new(f.clock.clone()),
                ByteBudget::Ordinary,
            )
            .unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        match transition {
            0 => {
                f.clock.advance_ms(15_000);
                f.low.join.collect_expired(&f.clock);
            }
            1 => f.low.lease.revoke(),
            _ => drop(f.low.join),
        }
        assert!(!action.is_pending());
        assert!(action.admit_engine(action.engine_bytes()).is_err());
        assert_eq!(f.pool.counts().1, 1);
        action.retire_if_unstarted();
        assert!(
            !action.is_settled(),
            "retirement cannot invent engine settlement"
        );
        action.settle_engine();
        assert_eq!(f.pool.counts().1, 0);
    }
}

#[test]
fn native_action_reserved_bytes_continue_under_ordinary_pressure_and_close_keeps_custody() {
    let mut f = fixture();
    let reserve = f
        .pool
        .reserve(f.low.context.scope.group, 32 * 1024)
        .unwrap();
    let pressure = f
        .pool
        .charge(
            f.low.context.scope.group,
            256 * 1024 - f.pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let action = f
        .low
        .join
        .derivation_action(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
            ByteBudget::Reserved(reserve.clone()),
        )
        .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    assert_eq!(f.pool.counts(), (256 * 1024, 1));
    reserve.close();
    let held = f.pool.counts();
    assert!(held.0 < 256 * 1024 && held.1 == 1);
    assert!(action.is_pending());
    let reader = action.input().unwrap();
    let CryptoInput::Derive(setup) = reader.as_input() else {
        panic!("derive input")
    };
    let payload_bytes = crypto_actions::derive_bytes(setup.transcript.len())
        .unwrap()
        .0;
    action.settle_engine();
    assert_eq!(f.pool.counts(), (held.0 - action.engine_bytes(), 0));
    assert!(action.input().is_err());
    drop(reader);
    assert_eq!(
        f.pool.counts(),
        (held.0 - action.engine_bytes() - payload_bytes, 0)
    );
    drop((action, f.low.join, pressure, reserve));
    // This test starts after actual public proof. It is not full-chain starvation acceptance.
}

#[test]
fn pending_seal_revokes_at_sign_but_exact_cached_submit_survives_construction_cutoff() {
    use catcoms_rt::RequestAuthority;
    use futures::FutureExt;
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let grant = f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    let mut revoked = Box::pin(grant.revoked());
    assert!(revoked.as_mut().now_or_never().is_none());
    f.clock.advance_ms(4900);
    let signed = f.low.sign(&flight, &f.clock).unwrap();
    assert!(!grant.authorized());
    assert!(revoked.as_mut().now_or_never().is_some());
    f.clock.advance_ms(200);
    assert_eq!(f.low.sign(&flight, &f.clock).unwrap(), signed);
    assert!(f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .is_err());
    assert!(
        f.low
            .join
            .source_packet_activity(
                &signed,
                &ca(&f.low.context, &f.low.lease),
                Arc::new(f.clock.clone()),
            )
            .is_ok(),
        "signing does not close actual source eligibility"
    );
}

#[test]
fn pending_proof_token_cannot_adopt_a_restarted_attempt_while_pair_derivation_stays_live() {
    use catcoms_rt::RequestAuthority;
    use futures::FutureExt;
    let mut f = fixture();
    let (hello, signed) = hello(&mut f);
    f.high
        .commit(signed, &hello.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.high.sign(&proof, &f.clock).unwrap();
    let ReceiveProgress::Decrypt(event) = f.low.receive(signed, &f.clock).unwrap() else {
        panic!("native proof event")
    };
    let derivation = f
        .low
        .join
        .pair_derivation_authority(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    let grant = f
        .low
        .join
        .incoming_crypto_authority(
            &event,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    let mut revoked = Box::pin(grant.revoked());
    assert!(revoked.as_mut().now_or_never().is_none());
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    assert!(!grant.authorized());
    assert!(revoked.as_mut().now_or_never().is_some());
    assert!(
        derivation.authorized(),
        "same pair transcript/private key survives a nonce restart"
    );
    assert!(
        f.low
            .join
            .incoming_crypto_authority(
                &event,
                &ca(&f.low.context, &f.low.lease),
                Arc::new(f.clock.clone()),
            )
            .is_err(),
        "a new capability cannot adopt the original token under a fresh attempt"
    );
}

#[test]
fn native_token_consume_revokes_pending_decrypt_without_retiring_unchanged_pair() {
    use catcoms_rt::RequestAuthority;
    let mut f = fixture();
    let (hello, signed) = hello(&mut f);
    let ReceiveProgress::Decrypt(event) = f.high.receive(signed, &f.clock).unwrap() else {
        panic!("native hello event")
    };
    let grant = f
        .high
        .join
        .incoming_crypto_authority(
            &event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    f.high
        .join
        .commit_confirmation(
            OpenedFlight {
                token: event.token,
                aad: event.aad(),
                sequence: event.header.sequence,
                ciphertext: event.ciphertext(),
                plaintext: &hello.plaintext,
            },
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    assert!(!grant.authorized());
    assert!(f.high.join.confirmations.pairs[&f.id].pin.is_live());
    assert!(f
        .high
        .join
        .incoming_crypto_authority(
            &event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
        )
        .is_err());
}

#[test]
fn abandoned_action_and_failed_replacement_reservation_cannot_restore_pending_crypto() {
    use crate::voice_v2::resources::ResourceKind;
    use catcoms_rt::RequestAuthority;
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    let old = f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    let pressure = f
        .pool
        .charge(
            f.low.context.scope.group,
            256 * 1024 - f.pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    f.low
        .join
        .abandon_confirmation(
            f.id,
            flight.action,
            flight.reservation.id,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    assert!(!old.authorized());
    assert!(matches!(
        f.low.prepare(f.id, &f.clock, &mut f.rng),
        Err(VoiceError::Capacity)
    ));
    assert!(!old.authorized());
    drop(pressure);
    let next = f.low.flight(f.id, &f.clock, &mut f.rng);
    assert_ne!(next.action, flight.action);
    assert!(next.reservation.sequence > flight.reservation.sequence);
    assert!(f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .is_err());
    assert!(f
        .low
        .join
        .confirmation_encryption_authority(
            &next,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap()
        .authorized());
}

#[test]
fn sdp_readiness_fits_actual_retained_three_flight_confirmation_without_repeating_transition() {
    use catcoms_rt::RequestAuthority;
    let mut f = fixture();
    assert!(f
        .low
        .join
        .prepare_sdp_exchange(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            &mut f.low.limits,
            &[],
            &f.clock
        )
        .is_err());
    establish(&mut f);
    let before = f.pool.counts();
    let owner = f
        .low
        .join
        .prepare_sdp_exchange(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            &mut f.low.limits,
            &[],
            &f.clock,
        )
        .unwrap();
    assert_eq!(
        f.pool.counts().0,
        // The peer row already exists from the challenged proof. Readiness additionally
        // prepays the local issuer's normal 2 KiB row before reserving the workflow.
        before.0 + super::super::super::negotiation::EXCHANGE_BYTES + 2048
    );
    assert!(f.pool.counts().0 <= 256 * 1024);
    let retry = f
        .low
        .join
        .prepare_sdp_exchange(
            f.id,
            &ca(&f.low.context, &f.low.lease),
            &mut f.low.limits,
            &[],
            &f.clock,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&owner, &retry));
    let reserved = f
        .low
        .join
        .ledger
        .reserve(
            f.low.signer.verifying_key().to_bytes(),
            ReserveRequest {
                handshake: f.id,
                kind: EncryptedKind::Offer,
                offer: None,
                ciphertext_bytes: MAX_SDP_PLAINTEXT + 16,
            },
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    let pending = f
        .low
        .join
        .packet_encryption_authority(
            &reserved,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    assert!(pending.authorized());
    assert!(f.low.join.confirmations.pairs[&f.id].established);
}

#[test]
fn established_application_packet_uses_its_own_crypto_deadline_after_confirmation_window() {
    use catcoms_rt::RequestAuthority;
    let mut f = fixture();
    establish(&mut f);
    let low_attempt = f.low.join.confirmations.pairs[&f.id].attempt.0.clone();
    f.clock.advance_ms(16_000);
    let reservation = f
        .low
        .join
        .ledger
        .reserve(
            f.low.signer.verifying_key().to_bytes(),
            ReserveRequest {
                handshake: f.id,
                kind: EncryptedKind::Ice,
                offer: Some(OfferId {
                    issuer: f.low.context.local,
                    message: [19; 32],
                }),
                ciphertext_bytes: 32,
            },
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    let grant = f
        .low
        .join
        .packet_encryption_authority(
            &reservation,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    assert!(grant.authorized() && low_attempt.is_live());
    f.clock.advance_ms(4999);
    assert!(grant.authorized());
    f.clock.advance_ms(1);
    assert!(!grant.authorized());
    assert!(f.low.join.confirmations.pairs[&f.id].pin.is_live());
}

#[test]
fn normal_three_flight_progress_preserves_each_accepted_attempt_generation() {
    let mut f = fixture();
    let (hello, signed) = hello(&mut f);
    let low = f.low.join.confirmations.pairs[&f.id].attempt.0.clone();
    f.high
        .commit(signed, &hello.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let high = f.high.join.confirmations.pairs[&f.id].attempt.0.clone();
    let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.high.sign(&proof, &f.clock).unwrap();
    f.low
        .commit(signed, &proof.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&finish, &f.clock).unwrap();
    f.high
        .commit(signed, &finish.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    assert!(low.is_live() && high.is_live());
    assert!(low.is_same(&f.low.join.confirmations.pairs[&f.id].attempt.0));
    assert!(high.is_same(&f.high.join.confirmations.pairs[&f.id].attempt.0));
}

#[test]
fn caller_changed_crypto_flight_data_cannot_obtain_current_native_authority() {
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let mut flight = f.low.flight(f.id, &f.clock, &mut f.rng);
    flight.plaintext[0] ^= 1;
    assert!(f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .is_err());
    flight.plaintext[0] ^= 1;
    flight.reservation.sequence += 1;
    assert!(f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .is_err());
    flight.reservation.sequence -= 1;
    flight.action += 1;
    assert!(f
        .low
        .join
        .confirmation_encryption_authority(
            &flight,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .is_err());
}

fn commit_event(
    end: &mut End,
    event: &InboundPacket,
    plaintext: &[u8],
    clock: &ManualClock,
    rng: &mut ChaCha20Rng,
) -> Result<(), VoiceError> {
    end.join.commit_confirmation(
        OpenedFlight {
            token: event.token,
            aad: event.aad(),
            sequence: event.header.sequence,
            ciphertext: event.ciphertext(),
            plaintext,
        },
        &ca(&end.context, &end.lease),
        clock,
        rng,
    )
}

#[test]
fn queued_hello_candidates_survive_lower_commit_and_reject_higher_commit_in_both_orders() {
    use crate::voice_v2::resources::ResourceKind;
    use catcoms_rt::RequestAuthority;
    use futures::FutureExt;
    for reverse in [false, true] {
        let mut f = fixture();
        let mut flights = Vec::new();
        let mut signed = Vec::new();
        for _ in 0..3 {
            let (flight, object) = hello(&mut f);
            flights.push(flight);
            signed.push(object);
        }
        let mut events = Vec::new();
        let mut grants = Vec::new();
        for object in &signed {
            let ReceiveProgress::Decrypt(event) = f.high.receive(object.clone(), &f.clock).unwrap()
            else {
                panic!("new hello candidate")
            };
            grants.push(
                f.high
                    .join
                    .incoming_crypto_authority(
                        &event,
                        &ca(&f.high.context, &f.high.lease),
                        Arc::new(f.clock.clone()),
                    )
                    .unwrap(),
            );
            events.push(event);
        }
        let mut newest_revoked = Box::pin(grants[2].revoked());
        let mut oldest_revoked = Box::pin(grants[0].revoked());
        assert!(newest_revoked.as_mut().now_or_never().is_none());
        assert!(oldest_revoked.as_mut().now_or_never().is_none());
        // A real shared native credit remains held even if an already-running engine later
        // returns stale plaintext. This unit still mocks AEAD, not actual browser computation.
        let engine = f
            .pool
            .charge(f.high.context.scope.group, 1024, ResourceKind::Crypto)
            .unwrap();
        let held = f.pool.counts();
        let first = if reverse { 2 } else { 1 };
        commit_event(
            &mut f.high,
            &events[first],
            &flights[first].plaintext,
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
        assert!(oldest_revoked.as_mut().now_or_never().is_some());
        if !reverse {
            assert!(grants[2].authorized());
            assert!(
                newest_revoked.as_mut().now_or_never().is_none(),
                "lower progress is a wake, not revocation"
            );
            assert!(f
                .high
                .join
                .incoming_crypto_authority(
                    &events[2],
                    &ca(&f.high.context, &f.high.lease),
                    Arc::new(f.clock.clone()),
                )
                .unwrap()
                .authorized());
            commit_event(
                &mut f.high,
                &events[2],
                &flights[2].plaintext,
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
        }
        assert!(newest_revoked.as_mut().now_or_never().is_some());
        assert!(!grants[1].authorized());
        let pair = &f.high.join.confirmations.pairs[&f.id];
        let before = (
            pair.starts,
            pair.action_serial,
            pair.low_nonce,
            pair.high_nonce,
        );
        assert!(commit_event(
            &mut f.high,
            &events[0],
            &flights[0].plaintext,
            &f.clock,
            &mut f.rng
        )
        .is_err());
        if reverse {
            assert_eq!(
                commit_event(
                    &mut f.high,
                    &events[1],
                    &flights[1].plaintext,
                    &f.clock,
                    &mut f.rng
                ),
                Err(VoiceError::Unauthorized)
            );
            assert!(
                matches!(
                    f.high.join.incoming_crypto_authority(
                        &events[1],
                        &ca(&f.high.context, &f.high.lease),
                        Arc::new(f.clock.clone()),
                    ),
                    Err(VoiceError::Unauthorized)
                ),
                "stale final commit did not consume its native token"
            );
        }
        assert!(matches!(
            f.high.receive(signed[2].clone(), &f.clock),
            Ok(ReceiveProgress::Retry(_))
        ));
        let pair = &f.high.join.confirmations.pairs[&f.id];
        assert_eq!(
            (
                pair.starts,
                pair.action_serial,
                pair.low_nonce,
                pair.high_nonce
            ),
            before
        );
        assert_eq!(
            f.pool.counts(),
            held,
            "stale/duplicate completion cannot refund actual engine or held payloads"
        );
        drop(engine);
        assert_eq!(f.pool.counts().1, held.1 - 1);
    }
}

#[test]
fn malformed_higher_hello_does_not_advance_native_candidate_floor() {
    use catcoms_rt::RequestAuthority;
    let mut f = fixture();
    let (lower, lower_signed) = hello(&mut f);
    let (higher, higher_signed) = hello(&mut f);
    let ReceiveProgress::Decrypt(low_event) = f.high.receive(lower_signed, &f.clock).unwrap()
    else {
        panic!("low hello")
    };
    let ReceiveProgress::Decrypt(high_event) = f.high.receive(higher_signed, &f.clock).unwrap()
    else {
        panic!("high hello")
    };
    let low = f
        .high
        .join
        .incoming_crypto_authority(
            &low_event,
            &ca(&f.high.context, &f.high.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    let mut malformed = higher.plaintext.clone();
    malformed[0] ^= 1;
    assert!(commit_event(&mut f.high, &high_event, &malformed, &f.clock, &mut f.rng).is_err());
    assert!(low.authorized());
    assert_eq!(f.high.join.confirmations.pairs[&f.id].starts, 0);
    commit_event(
        &mut f.high,
        &low_event,
        &lower.plaintext,
        &f.clock,
        &mut f.rng,
    )
    .unwrap();
    assert_eq!(f.high.join.confirmations.pairs[&f.id].starts, 1);
}

#[test]
fn native_confirmation_supersession_revokes_actual_queued_source_actions_without_refunding_lower_work(
) {
    use crate::voice_v2::helper::{BoundRoute, RoomAuthority};
    use crate::voice_v2::runtime::{Runtime, View};
    use crate::voice_v2::source::SourceOutcome;
    for transition in 0..3 {
        let clock = ManualClock::new(1000);
        let runtime = Runtime::new(Arc::new(clock.clone()));
        let mut f = fixture_with_resources(clock, runtime.resources());
        let (hello_flight, signed_hello) = hello(&mut f);
        let queued = if transition == 1 {
            f.high
                .commit(
                    signed_hello.clone(),
                    &hello_flight.plaintext,
                    &f.clock,
                    &mut f.rng,
                )
                .unwrap();
            let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
            f.high.sign(&proof, &f.clock).unwrap()
        } else {
            signed_hello.clone()
        };
        let end = if transition == 1 { &f.high } else { &f.low };
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        let (mut actor, registration) = runtime
            .prepare_actor(end.context.scope.group, end.context.local, &ticket)
            .unwrap();
        registration.register().unwrap();
        let bridge = DeviceKeypair::from_seed(&[3; 32]).device_id();
        actor
            .sync_view(View {
                epoch: end.context.epoch,
                members: [f.low.context.local, f.high.context.local, bridge]
                    .into_iter()
                    .collect(),
                policy: end.context.scope.policy,
                eligible: true,
                indirect: true,
            })
            .unwrap();
        let source = actor.source().unwrap();
        let activity = end
            .join
            .source_packet_activity(&queued, &ca(&end.context, &end.lease), runtime.clock())
            .unwrap();
        let room = RoomAuthority {
            scope: end.context.scope,
            lease: CallLease::default(),
            custody: None,
        };
        source
            .admit(&actor, queued.clone(), activity.clone(), room.clone())
            .unwrap();
        let routes = [BoundRoute {
            device: bridge,
            peer: catcoms_rt::PeerId::from_u64(3),
        }];
        let job = source.take_ready(&actor, &routes).unwrap().pop().unwrap();
        let lower = job.keepalive();
        match transition {
            0 => {
                f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
            }
            1 => {
                f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
                let next = f.low.flight(f.id, &f.clock, &mut f.rng);
                let signed = f.low.sign(&next, &f.clock).unwrap();
                f.high
                    .commit(signed, &next.plaintext, &f.clock, &mut f.rng)
                    .unwrap();
            }
            _ => {
                f.high
                    .commit(signed_hello, &hello_flight.plaintext, &f.clock, &mut f.rng)
                    .unwrap();
                let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
                let signed = f.high.sign(&proof, &f.clock).unwrap();
                f.low
                    .commit(signed, &proof.plaintext, &f.clock, &mut f.rng)
                    .unwrap();
            }
        }
        assert!(
            !job.authority().authorized(),
            "native transition {transition}"
        );
        assert_eq!(
            source.admit(&actor, queued, activity, room),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(source.counts(), (1, 1, 1));
        assert!(source.take_ready(&actor, &routes).unwrap().is_empty());
        let after_transition = runtime.resources().counts();
        assert_eq!(
            source
                .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
                .unwrap(),
            SourceOutcome::Retired
        );
        assert_eq!(runtime.resources().counts(), after_transition);
        drop(lower);
        assert!(runtime.resources().counts().0 < after_transition.0);
        let end = if transition == 1 {
            &mut f.high
        } else {
            &mut f.low
        };
        let next = end.flight(f.id, &f.clock, &mut f.rng);
        let signed = end.sign(&next, &f.clock).unwrap();
        assert!(end
            .join
            .source_packet_activity(&signed, &ca(&end.context, &end.lease), runtime.clock())
            .is_ok());
    }
}
#[test]
fn queued_native_confirmation_expires_at_original_deadline_without_endpoint_activity() {
    use crate::voice_v2::helper::{BoundRoute, RoomAuthority};
    use crate::voice_v2::runtime::{Runtime, View};
    use crate::voice_v2::source::SourceOutcome;
    let clock = ManualClock::new(1000);
    let runtime = Runtime::new(Arc::new(clock.clone()));
    let mut f = fixture_with_resources(clock, runtime.resources());
    // A late first flight is still valid when signed, but must not buy another 15 seconds.
    f.clock.advance_ms(14_000);
    let (_, signed) = hello(&mut f);
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(f.low.context.scope.group, f.low.context.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    let bridge = DeviceKeypair::from_seed(&[3; 32]).device_id();
    actor
        .sync_view(View {
            epoch: f.low.context.epoch,
            members: [f.low.context.local, f.high.context.local, bridge]
                .into_iter()
                .collect(),
            policy: f.low.context.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let source = actor.source().unwrap();
    let activity = f
        .low
        .join
        .source_packet_activity(&signed, &ca(&f.low.context, &f.low.lease), runtime.clock())
        .unwrap();
    let room = RoomAuthority {
        scope: f.low.context.scope,
        lease: CallLease::default(),
        custody: None,
    };
    source
        .admit(&actor, signed.clone(), activity.clone(), room.clone())
        .unwrap();
    let routes = [BoundRoute {
        device: bridge,
        peer: catcoms_rt::PeerId::from_u64(3),
    }];
    let job = source.take_ready(&actor, &routes).unwrap().pop().unwrap();
    let lower = job.keepalive();
    assert!(job.authority().authorized());
    f.clock.advance_ms(1000);
    assert!(f.clock.now_ms() < signed.expires());
    // No LocalJoin method or maintenance pass has run: the pair pin remains live, so the
    // captured deadline itself must fence both detached work and a cached admission retry.
    assert!(f.low.join.confirmations.pairs[&f.id].pin.is_live());
    assert!(!job.authority().authorized());
    assert_eq!(
        source.admit(&actor, signed.clone(), activity, room),
        Err(VoiceError::Unauthorized)
    );
    assert!(matches!(
        f.low.join.source_packet_activity(
            &signed,
            &ca(&f.low.context, &f.low.lease),
            runtime.clock()
        ),
        Err(VoiceError::Unauthorized)
    ));
    assert!(source.take_ready(&actor, &routes).unwrap().is_empty());
    let held = runtime.resources().counts();
    assert_eq!(
        source
            .complete(job.finish(Some(crate::VoiceTransportStatus::HelperQueued)))
            .unwrap(),
        SourceOutcome::Retired
    );
    assert_eq!(runtime.resources().counts(), held);
    drop(lower);
    assert!(runtime.resources().counts().0 < held.0);
}

#[test]
fn collection_retires_idle_confirming_pair_pin_without_recreating_pair_or_action_serials() {
    let mut f = fixture();
    let _ = hello(&mut f);
    let generation = f.low.join.confirmations.pairs[&f.id].generation;
    let action = f.low.join.confirmations.pairs[&f.id].action_serial;
    let pin = f.low.join.confirmations.pairs[&f.id].pin.clone();
    f.clock.advance_ms(15_000);
    f.low.join.collect_expired(&f.clock);
    assert!(!pin.is_live());
    let pair = &f.low.join.confirmations.pairs[&f.id];
    assert!(pair.retired && pair.action.is_none());
    assert_eq!(pair.generation, generation);
    assert_eq!(pair.action_serial, action);
    f.clock.set_wall_ms(1000);
    f.low.join.collect_expired(&f.clock);
    assert_eq!(
        f.low.start(f.id, &f.clock, &mut f.rng),
        Err(VoiceError::Unauthorized)
    );
}

fn through_proof(f: &mut Fixture) -> (CryptoFlight, SignedObject, CryptoFlight, SignedObject) {
    let (hello, signed_hello) = hello(f);
    f.high
        .commit(signed_hello.clone(), &hello.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
    let signed_proof = f.high.sign(&proof, &f.clock).unwrap();
    f.low
        .commit(signed_proof.clone(), &proof.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    (hello, signed_hello, proof, signed_proof)
}
fn establish(f: &mut Fixture) {
    through_proof(f);
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&finish, &f.clock).unwrap();
    f.high
        .commit(signed, &finish.plaintext, &f.clock, &mut f.rng)
        .unwrap();
}

#[test]
fn three_native_flights_establish_only_after_current_finish_and_duplicates_are_cached() {
    let mut f = fixture();
    assert!(!f.low.established(f.id, &f.clock));
    assert!(!f.high.established(f.id, &f.clock));
    assert_eq!(
        f.high.start(f.id, &f.clock, &mut f.rng),
        Err(VoiceError::Unauthorized)
    );
    let (_, hello, _, proof) = through_proof(&mut f);
    assert!(!f.low.established(f.id, &f.clock));
    assert!(!f.high.established(f.id, &f.clock));
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed_finish = f.low.sign(&finish, &f.clock).unwrap();
    assert!(f.low.established(f.id, &f.clock));
    assert!(!f.high.established(f.id, &f.clock));
    assert!(matches!(f.low.receive(proof, &f.clock), Ok(ReceiveProgress::Retry(id)) if id == f.id));
    let Some(ConfirmationOutput::Send(retry)) = f.low.prepare(f.id, &f.clock, &mut f.rng).unwrap()
    else {
        panic!("cached final flight")
    };
    assert_eq!(*retry, signed_finish);
    f.high
        .commit(
            signed_finish.clone(),
            &finish.plaintext,
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    assert!(f.high.established(f.id, &f.clock));
    assert!(matches!(
        f.high.receive(signed_finish, &f.clock),
        Ok(ReceiveProgress::Confirmed)
    ));
    assert!(
        matches!(f.high.receive(hello, &f.clock), Ok(ReceiveProgress::Retry(id)) if id == f.id)
    );
}

#[test]
fn valid_progress_survives_response_capacity_without_new_nonce_or_second_decrypt() {
    for at_finish in [false, true] {
        let mut f = fixture();
        let (hello, signed_hello) = hello(&mut f);
        let (receiver, object, plain) = if at_finish {
            f.high
                .commit(signed_hello, &hello.plaintext, &f.clock, &mut f.rng)
                .unwrap();
            let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
            let signed = f.high.sign(&proof, &f.clock).unwrap();
            (&mut f.low, signed, proof.plaintext)
        } else {
            (&mut f.high, signed_hello, hello.plaintext)
        };
        receiver
            .commit(object.clone(), &plain, &f.clock, &mut f.rng)
            .unwrap();
        let nonce = receiver.join.confirmations.pairs[&f.id].high_nonce;
        let bytes = f.pool.counts().0;
        let held = f
            .pool
            .charge(
                receiver.context.scope.group,
                256 * 1024 - bytes,
                ResourceKind::Retained,
            )
            .unwrap();
        assert!(matches!(
            receiver.prepare(f.id, &f.clock, &mut f.rng),
            Err(VoiceError::Capacity)
        ));
        assert!(matches!(
            receiver.receive(object, &f.clock),
            Ok(ReceiveProgress::Retry(_))
        ));
        assert_eq!(receiver.join.confirmations.pairs[&f.id].high_nonce, nonce);
        drop(held);
        receiver.flight(f.id, &f.clock, &mut f.rng);
    }
}

#[test]
fn superseded_proof_and_finish_crypto_can_neither_sign_nor_establish() {
    let mut f = fixture();
    let (hello1, signed1) = hello(&mut f);
    f.high
        .commit(signed1.clone(), &hello1.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let delayed_proof1 = f.high.flight(f.id, &f.clock, &mut f.rng);
    let (hello2, signed2) = hello(&mut f);
    f.high
        .commit(signed2, &hello2.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    assert_eq!(
        f.high.sign(&delayed_proof1, &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    assert!(matches!(
        f.high.receive(signed1, &f.clock),
        Err(VoiceError::Replay)
    ));
    let proof2 = f.high.flight(f.id, &f.clock, &mut f.rng);
    let signed_proof2 = f.high.sign(&proof2, &f.clock).unwrap();
    f.low
        .commit(signed_proof2, &proof2.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let delayed_finish2 = f.low.flight(f.id, &f.clock, &mut f.rng);
    let _ = hello(&mut f); // Third fresh native attempt supersedes the pending Finish.
    assert_eq!(
        f.low.sign(&delayed_finish2, &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    assert!(!f.low.established(f.id, &f.clock));
    assert!(!f.high.established(f.id, &f.clock));
}

#[test]
fn failed_engine_and_expired_construction_burn_reservations_but_preserve_confirmation_nonce() {
    let mut f = fixture();
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let first = f.low.flight(f.id, &f.clock, &mut f.rng);
    f.clock.advance_ms(5000);
    // A discarded prepare reply recovers without knowing its old action/reservation ids.
    let expired_replacement = f.low.flight(f.id, &f.clock, &mut f.rng);
    f.low
        .join
        .abandon_confirmation(
            f.id,
            expired_replacement.action,
            expired_replacement.reservation.id,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    let next = f.low.flight(f.id, &f.clock, &mut f.rng);
    assert_eq!(first.plaintext, next.plaintext);
    assert_eq!(next.reservation.sequence, first.reservation.sequence + 2);
    assert_eq!(
        f.low.sign(&first, &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    f.low.sign(&next, &f.clock).unwrap();
    assert!(f
        .low
        .join
        .abandon_confirmation(
            f.id,
            next.action,
            next.reservation.id,
            &ca(&f.low.context, &f.low.lease),
            &f.clock
        )
        .is_err());
}

#[test]
fn lost_prepare_reply_recovers_through_exact_input_retry_after_expiry_and_capacity() {
    for at_finish in [false, true] {
        let mut f = fixture();
        let (hello, signed_hello) = hello(&mut f);
        let (receiver, object, plain) = if at_finish {
            f.high
                .commit(signed_hello, &hello.plaintext, &f.clock, &mut f.rng)
                .unwrap();
            let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
            let signed = f.high.sign(&proof, &f.clock).unwrap();
            (&mut f.low, signed, proof.plaintext)
        } else {
            (&mut f.high, signed_hello, hello.plaintext)
        };
        receiver
            .commit(object.clone(), &plain, &f.clock, &mut f.rng)
            .unwrap();
        let lost = receiver.flight(f.id, &f.clock, &mut f.rng);
        let bytes = f.pool.counts().0;
        let held = f
            .pool
            .charge(
                receiver.context.scope.group,
                256 * 1024 - bytes,
                ResourceKind::Retained,
            )
            .unwrap();
        f.clock.advance_ms(5000);
        assert!(matches!(
            receiver.receive(object, &f.clock),
            Ok(ReceiveProgress::Retry(_))
        ));
        assert!(matches!(
            receiver.prepare(f.id, &f.clock, &mut f.rng),
            Err(VoiceError::Capacity)
        ));
        drop(held);
        // Recovery uses only the pair identity; no old action/reservation id is supplied.
        let recovered = receiver.flight(f.id, &f.clock, &mut f.rng);
        assert_eq!(recovered.plaintext, lost.plaintext);
        assert_eq!(
            recovered.reservation.sequence,
            lost.reservation.sequence + 1
        );
        assert!(
            f.pool.counts().0 > bytes,
            "the old packet credit was not refunded"
        );
        assert_eq!(
            receiver.sign(&lost, &f.clock).err(),
            Some(VoiceError::Reservation)
        );
        receiver.sign(&recovered, &f.clock).unwrap();
    }
}

#[test]
fn malformed_and_reflected_payloads_never_create_a_response_entry() {
    for dimension in 0..4 {
        let mut f = fixture();
        let (hello, signed) = hello(&mut f);
        let mut bytes = hello.plaintext.clone();
        match dimension {
            0 => bytes[0] ^= 1,
            1 => bytes[CONFIRM_DOMAIN.len() + 4 + 1] ^= 1,
            2 => {
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
            }
            _ => bytes.push(0),
        }
        // Nonce changes on Hello are legal for its original initiator, so use a wrong role
        // instead of alleging that the responder can know an unseen native initiator nonce.
        if dimension == 2 {
            bytes = plaintext(EncryptedKind::ConfirmProof, f.id, [2; 32], Some([3; 32]));
        }
        assert!(f
            .high
            .commit(signed.clone(), &bytes, &f.clock, &mut f.rng)
            .is_err());
        assert!(f.high.join.confirmations.pairs[&f.id].action.is_none());
        assert!(!matches!(
            f.high.receive(signed, &f.clock),
            Ok(ReceiveProgress::Retry(_))
        ));
        assert!(!f.high.established(f.id, &f.clock));
    }
}

#[test]
fn proof_and_finish_must_echo_the_exact_current_native_nonces() {
    let mut f = fixture();
    let (hello, signed) = hello(&mut f);
    f.high
        .commit(signed, &hello.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.high.sign(&proof, &f.clock).unwrap();
    let (mut low, high) = decode(&proof.plaintext, EncryptedKind::ConfirmProof, f.id).unwrap();
    low[0] ^= 1;
    let wrong = plaintext(EncryptedKind::ConfirmProof, f.id, low, high);
    assert_eq!(
        f.low.commit(signed, &wrong, &f.clock, &mut f.rng),
        Err(VoiceError::Unauthorized)
    );
    assert!(!f.low.established(f.id, &f.clock));

    let mut f = fixture();
    through_proof(&mut f);
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&finish, &f.clock).unwrap();
    let (low, high) = decode(&finish.plaintext, EncryptedKind::ConfirmFinish, f.id).unwrap();
    let mut high = high.unwrap();
    high[0] ^= 1;
    let wrong = plaintext(EncryptedKind::ConfirmFinish, f.id, low, Some(high));
    assert_eq!(
        f.high.commit(signed, &wrong, &f.clock, &mut f.rng),
        Err(VoiceError::Unauthorized)
    );
    assert!(!f.high.established(f.id, &f.clock));
}

fn replacement_high(f: &mut Fixture) -> End {
    let seed = if DeviceKeypair::from_seed(&[1; 32]).device_id() == f.high.context.local {
        1
    } else {
        2
    };
    let mut end = End::new(seed, f.pool.clone(), &mut f.rng);
    end.lease = CallLease::default();
    end.join = LocalJoin::new(&end.context, key(9), end.lease.clone(), f.pool.clone()).unwrap();
    end
}

#[test]
fn successful_newer_promotion_retires_old_pair_and_captured_old_finish() {
    for old_established in [false, true] {
        let mut f = fixture();
        through_proof(&mut f);
        let captured = f.low.flight(f.id, &f.clock, &mut f.rng);
        let old = f.id;
        if old_established {
            let signed = f.low.sign(&captured, &f.clock).unwrap();
            f.high
                .commit(signed, &captured.plaintext, &f.clock, &mut f.rng)
                .unwrap();
        }
        let mut replacement = replacement_high(&mut f);
        let newer = admit_pair(&mut f.low, &mut replacement, &f.clock, &mut f.rng);
        let _old_high = std::mem::replace(&mut f.high, replacement);
        f.id = newer;
        establish(&mut f);
        assert!(f.low.established(newer, &f.clock));
        assert_eq!(
            f.low
                .join
                .pair_established(old, &ca(&f.low.context, &f.low.lease), &f.clock),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(
            f.low.sign(&captured, &f.clock).err(),
            Some(VoiceError::Unauthorized)
        );
    }
}

#[tokio::test]
async fn deferred_finish_worker_cannot_submit_after_a_new_native_attempt() {
    let mut f = fixture();
    through_proof(&mut f);
    let delayed = f.low.flight(f.id, &f.clock, &mut f.rng);
    let owner = Arc::new(std::sync::Mutex::new(f.low));
    let worker_owner = owner.clone();
    let worker_clock = f.clock.clone();
    let (release, wait) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        wait.await.unwrap();
        worker_owner.lock().unwrap().sign(&delayed, &worker_clock)
    });
    owner
        .lock()
        .unwrap()
        .start(f.id, &f.clock, &mut f.rng)
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(worker.await.unwrap().err(), Some(VoiceError::Reservation));
    assert!(!owner.lock().unwrap().established(f.id, &f.clock));
}

#[tokio::test]
async fn deferred_proof_worker_cannot_submit_after_a_new_native_hello() {
    let mut f = fixture();
    let (hello1, signed1) = hello(&mut f);
    f.high
        .commit(signed1, &hello1.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let delayed = f.high.flight(f.id, &f.clock, &mut f.rng);
    let owner = Arc::new(std::sync::Mutex::new(f.high));
    let worker_owner = owner.clone();
    let worker_clock = f.clock.clone();
    let (release, wait) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        wait.await.unwrap();
        worker_owner.lock().unwrap().sign(&delayed, &worker_clock)
    });
    f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    let hello2 = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed2 = f.low.sign(&hello2, &f.clock).unwrap();
    owner
        .lock()
        .unwrap()
        .commit(signed2, &hello2.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(worker.await.unwrap().err(), Some(VoiceError::Reservation));
    assert!(!owner.lock().unwrap().established(f.id, &f.clock));
}

#[test]
fn renewal_and_watcher_replacement_do_not_revoke_an_admitted_pair() {
    let mut f = fixture();
    let (hello, signed) = hello(&mut f);
    f.clock.advance_ms(5000);
    let ann = f.low.announce(&f.clock, &mut f.rng);
    let challenge = f.high.challenge(&ann, &f.clock, &mut f.rng);
    let proof = f.low.answer(&challenge, &f.clock, &mut f.rng);
    let same = f.high.begin(&proof, &f.clock);
    assert_eq!(same.handshake, f.id);
    f.high.watch_lease.revoke(); // Pair authority is a separate native owner, not a roster clone.
    f.high
        .commit(signed, &hello.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let proof = f.high.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.high.sign(&proof, &f.clock).unwrap();
    f.low
        .commit(signed, &proof.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&finish, &f.clock).unwrap();
    f.high
        .commit(signed, &finish.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    assert!(f.low.established(f.id, &f.clock));
    assert!(f.high.established(f.id, &f.clock));
}

#[test]
fn removal_and_lock_permanently_reject_captured_engine_results() {
    let mut f = fixture();
    let _ = through_proof(&mut f);
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    f.low.join.retire_member(f.high.context.local);
    assert_eq!(
        f.low.sign(&finish, &f.clock).err(),
        Some(VoiceError::Unauthorized)
    );
    let mut f = fixture();
    let _ = through_proof(&mut f);
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    f.low.lease.revoke();
    assert_eq!(
        f.low.sign(&finish, &f.clock).err(),
        Some(VoiceError::Unauthorized)
    );
}

#[test]
fn membership_loss_inside_ledger_commit_also_revokes_confirmed_and_cached_paths() {
    let mut f = fixture();
    let (_, _, _, proof) = through_proof(&mut f);
    let Body::Encrypted {
        mut header,
        ciphertext,
    } = proof.body().clone()
    else {
        unreachable!()
    };
    header.sequence += 1;
    let another = SignedObject::issue(
        proof.scope(),
        &f.high.signer,
        [55; 32],
        proof.issued(),
        proof.expires(),
        Body::Encrypted { header, ciphertext },
    )
    .unwrap();
    let ReceiveProgress::Decrypt(pending) = f.low.receive(another, &f.clock).unwrap() else {
        panic!("second admitted flight")
    };
    let finish = f.low.flight(f.id, &f.clock, &mut f.rng);
    let signed = f.low.sign(&finish, &f.clock).unwrap();
    f.high
        .commit(signed, &finish.plaintext, &f.clock, &mut f.rng)
        .unwrap();
    assert!(f.low.established(f.id, &f.clock));
    let removed = |id| id != f.high.context.local;
    let authority = Authority {
        context: &f.low.context,
        lease: &f.low.lease,
        member: &removed,
    };
    let plaintext = f.high.join.confirmations.pairs[&f.id]
        .action
        .as_ref()
        .unwrap()
        .plaintext
        .clone();
    assert_eq!(
        f.low.join.commit_confirmation(
            OpenedFlight {
                token: pending.token,
                aad: pending.aad(),
                sequence: pending.header.sequence,
                ciphertext: pending.ciphertext(),
                plaintext: &plaintext
            },
            &authority,
            &f.clock,
            &mut f.rng
        ),
        Err(VoiceError::Unauthorized)
    );
    // Membership is current again, but every native path observes the SAME revoked pair lease.
    assert_eq!(
        f.low
            .join
            .pair_setup(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.low
            .join
            .pair_established(f.id, &ca(&f.low.context, &f.low.lease), &f.clock),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.low.receive(proof, &f.clock).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(
        f.low.prepare(f.id, &f.clock, &mut f.rng).err(),
        Some(VoiceError::Unauthorized)
    );
}

#[test]
fn three_start_limit_and_original_monotonic_deadline_cannot_be_refilled_by_clock_changes() {
    let mut f = fixture();
    for _ in 0..3 {
        f.low.start(f.id, &f.clock, &mut f.rng).unwrap();
    }
    assert_eq!(
        f.low.start(f.id, &f.clock, &mut f.rng),
        Err(VoiceError::RateLimited)
    );
    f.clock.advance_ms(15_000);
    f.clock.set_wall_ms(1000);
    assert_eq!(
        f.low.start(f.id, &f.clock, &mut f.rng),
        Err(VoiceError::Freshness)
    );
    assert!(f.low.join.confirmations.pairs[&f.id].retired);
}

#[test]
fn newer_join_waits_for_key_proof_and_failed_confirmation_preserves_old_pair() {
    let mut f = fixture();
    establish(&mut f);
    let old = f.id;
    let new_lease = CallLease::default();
    let mut replacement =
        LocalJoin::new(&f.high.context, key(9), new_lease.clone(), f.pool.clone()).unwrap();
    let ann = replacement
        .announce(
            &f.high.signer,
            &ca(&f.high.context, &new_lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    let challenge = f.low.challenge(&ann, &f.clock, &mut f.rng);
    let proof = replacement
        .answer(
            &challenge,
            &f.high.signer,
            &ca(&f.high.context, &new_lease),
            &f.clock,
            &mut f.rng,
        )
        .unwrap();
    let newer = f.low.begin(&proof, &f.clock).handshake;
    assert_ne!(newer, old);
    assert!(f.low.established(old, &f.clock));
    f.low.start(newer, &f.clock, &mut f.rng).unwrap();
    // Failing/abandoning this candidate leaves the working old native pair intact.
    f.clock.advance_ms(15_000);
    assert!(f.low.prepare(newer, &f.clock, &mut f.rng).is_err());
    assert!(f.low.established(old, &f.clock));
}
