//! Actual protected result transactions. Native signatures and actor admission are real; the
//! supplied AEAD output remains the trusted-output fixture used by the surrounding tests.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn protected_seal(
    end: &mut End,
    actor: &ActorOwner,
    action: &Arc<CryptoAction>,
    clock: &ManualClock,
) -> (SignedObject, Vec<u8>) {
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("native seal")
    };
    let plain = flight.plaintext.clone();
    let ciphertext = vec![22; plain.len() + 16];
    action.admit_engine(action.engine_bytes()).unwrap();
    let room = room(end);
    let receipt = end
        .join
        .submit_protected_confirmation_result(
            ProtectedSealedResult {
                actor,
                room: &room,
                action,
                ciphertext: &ciphertext,
                claimed_tag: action.output_tag(&ciphertext).unwrap(),
            },
            &end.signer,
            &ca(&end.context, &end.lease),
            clock,
            |_, _| Ok(()),
        )
        .unwrap();
    // Recover the already-published bytes before ACK; this compatibility lookup cannot sign a
    // second packet or replace the native Root-origin first transition above.
    let signed = end
        .join
        .submit_confirmation_action(
            action,
            &ciphertext,
            &end.signer,
            &ca(&end.context, &end.lease),
            clock,
        )
        .unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    (signed, plain)
}

struct SignProbe<'a> {
    signer: &'a DeviceKeypair,
    calls: &'a AtomicUsize,
    action: &'a CryptoAction,
    settle: bool,
}
impl VoiceSigner for SignProbe<'_> {
    fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
        self.signer.voice_public_key()
    }
    fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError> {
        assert!(self.action.result_fence_available_for_test());
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.settle {
            self.action.settle_engine();
            assert!(
                !self.action.is_settled(),
                "actual signature preparation is still running"
            );
            assert!(self.action.input().is_ok());
        }
        self.signer.sign_voice(bytes)
    }
}

#[test]
fn protected_seal_capacity_retry_reuses_one_signature_and_never_publishes_a_refused_packet() {
    let mut p = fixture();
    let action = start(&mut p, None);
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("seal")
    };
    let bytes = vec![22; flight.plaintext.len() + 16];
    let tag = action.output_tag(&bytes).unwrap();
    let room = room(&p.f.low);
    let calls = AtomicUsize::new(0);
    let signer = SignProbe {
        signer: &p.f.low.signer,
        action: &action,
        calls: &calls,
        settle: false,
    };
    action.admit_engine(action.engine_bytes()).unwrap();
    let request = || ProtectedSealedResult {
        actor: &p.low,
        room: &room,
        action: &action,
        ciphertext: &bytes,
        claimed_tag: tag,
    };
    assert_eq!(
        p.f.low
            .join
            .submit_protected_confirmation_result(
                request(),
                &signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| Err(VoiceError::Capacity)
            )
            .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .action
        .as_ref()
        .unwrap()
        .signed
        .is_none());
    let prepared =
        p.f.low
            .join
            .ledger
            .prepare_sealed_publication(
                flight.reservation.id,
                &bytes,
                &signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
            )
            .unwrap();
    let private_signed = prepared.signed().clone();
    drop(prepared);
    assert!(p
        .f
        .low
        .join
        .source_packet_activity(
            &private_signed,
            &ca(&p.f.low.context, &p.f.low.lease),
            Arc::new(p.f.clock.clone())
        )
        .is_err());
    let mut changed = bytes.clone();
    changed[0] ^= 1;
    assert_eq!(
        p.f.low
            .join
            .submit_protected_confirmation_result(
                ProtectedSealedResult {
                    ciphertext: &changed,
                    claimed_tag: action.output_tag(&changed).unwrap(),
                    ..request()
                },
                &signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| panic!("changed body")
            )
            .err(),
        Some(VoiceError::RetryMismatch)
    );
    let receipt =
        p.f.low
            .join
            .submit_protected_confirmation_result(
                request(),
                &signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| Ok(()),
            )
            .unwrap();
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(matches!(receipt.outcome, CryptoOutcome::Sealed { .. }));
    assert!(p
        .f
        .low
        .join
        .source_packet_activity(
            &private_signed,
            &ca(&p.f.low.context, &p.f.low.lease),
            Arc::new(p.f.clock.clone())
        )
        .is_ok());
    action.settle_engine();
}

#[test]
fn protected_seal_settlement_during_signing_cannot_publish_or_refund_running_preparation() {
    let mut p = fixture();
    let action = start(&mut p, None);
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("seal")
    };
    let bytes = vec![22; flight.plaintext.len() + 16];
    let calls = AtomicUsize::new(0);
    let signer = SignProbe {
        signer: &p.f.low.signer,
        action: &action,
        calls: &calls,
        settle: true,
    };
    let room = room(&p.f.low);
    action.admit_engine(action.engine_bytes()).unwrap();
    assert_eq!(
        p.f.low
            .join
            .submit_protected_confirmation_result(
                ProtectedSealedResult {
                    actor: &p.low,
                    room: &room,
                    action: &action,
                    ciphertext: &bytes,
                    claimed_tag: action.output_tag(&bytes).unwrap()
                },
                &signer,
                &ca(&p.f.low.context, &p.f.low.lease),
                &p.f.clock,
                |_, _| panic!("settled before final admission")
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(action.is_settled());
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .action
        .as_ref()
        .unwrap()
        .signed
        .is_none());
}

#[test]
fn protected_seal_atomic_session_close_wins_before_publication_with_live_native_child() {
    let mut p = fixture();
    let action = start(&mut p, None);
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("seal")
    };
    let bytes = vec![22; flight.plaintext.len() + 16];
    let room = room(&p.f.low);
    action.admit_engine(action.engine_bytes()).unwrap();
    let child = p.low.child_lease().unwrap();
    let closed = p.low.revocation_observer_for_test();
    let runtime = p._runtimes[0].clone();
    let mut revoker = None;
    let result = p.f.low.join.submit_protected_confirmation_result(
        ProtectedSealedResult {
            actor: &p.low,
            room: &room,
            action: &action,
            ciphertext: &bytes,
            claimed_tag: action.output_tag(&bytes).unwrap(),
        },
        &p.f.low.signer,
        &ca(&p.f.low.context, &p.f.low.lease),
        &p.f.clock,
        |_, _| {
            revoker = Some(std::thread::spawn(move || runtime.revoke_session()));
            futures::executor::block_on(closed.wait_revoked());
            assert!(child.is_live());
            Ok(())
        },
    );
    revoker.unwrap().join().unwrap();
    assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    assert!(!child.is_live());
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.low.join.confirmations.pairs[&p.f.id]
        .action
        .as_ref()
        .unwrap()
        .signed
        .is_none());
    action.settle_engine();
}

#[test]
fn protected_seal_last_native_clock_rechecks_each_original_owner() {
    for owner in ["call", "pair", "chain", "room", "actor"] {
        let mut p = fixture();
        let action = start(&mut p, None);
        let input = action.input().unwrap();
        let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
            panic!("seal")
        };
        let bytes = vec![22; flight.plaintext.len() + 16];
        let room = RoomAuthority {
            lease: CallLease::default(),
            ..room(&p.f.low)
        };
        action.admit_engine(action.engine_bytes()).unwrap();
        let pair = &p.f.low.join.confirmations.pairs[&p.f.id];
        let close = match owner {
            "call" => p.f.low.lease.clone(),
            "pair" => pair.pin.clone(),
            "chain" => pair.chain.as_ref().unwrap().lease(),
            "room" => room.lease.clone(),
            _ => p.low.revocation_observer_for_test(),
        };
        let last = p.origin_clock.reads.load(Ordering::Acquire) + 2;
        *p.origin_clock.revoke.lock().unwrap() = Some((last, close));
        assert_eq!(
            p.f.low
                .join
                .submit_protected_confirmation_result(
                    ProtectedSealedResult {
                        actor: &p.low,
                        room: &room,
                        action: &action,
                        ciphertext: &bytes,
                        claimed_tag: action.output_tag(&bytes).unwrap()
                    },
                    &p.f.low.signer,
                    &ca(&p.f.low.context, &p.f.low.lease),
                    &p.f.clock,
                    |_, _| Ok(())
                )
                .err(),
            Some(VoiceError::Unauthorized),
            "{owner}"
        );
        assert!(action.terminal_status().receipt.is_none(), "{owner}");
        assert!(
            p.f.low.join.confirmations.pairs[&p.f.id]
                .action
                .as_ref()
                .unwrap()
                .signed
                .is_none(),
            "{owner}"
        );
        action.settle_engine();
    }
}

fn checked(
    end: &mut End,
    actor: &ActorOwner,
    result: (&Arc<CryptoAction>, &[u8]),
    clock: &dyn Clock,
    rng: &mut ChaCha20Rng,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<CryptoResultReceipt, VoiceError> {
    let room = room(end);
    end.join.commit_protected_confirmation_result(
        ProtectedOpenedResult {
            actor,
            room: &room,
            action: result.0,
            plaintext: result.1,
            claimed_tag: result.0.output_tag(result.1).unwrap(),
        },
        &ca(&end.context, &end.lease),
        clock,
        rng,
        final_owner,
    )
}

fn opened(
    end: &mut End,
    actor: &ActorOwner,
    action: &Arc<CryptoAction>,
    bytes: &[u8],
    clock: &ManualClock,
    rng: &mut ChaCha20Rng,
) -> CryptoResultReceipt {
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = checked(end, actor, (action, bytes), clock, rng, |_, _| Ok(())).unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    receipt
}

#[test]
fn protected_origin_result_progresses_each_role_and_recovers_closed_rejections() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, hello_plain) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, high_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    assert_eq!(
        opened(
            &mut p.f.high,
            &p.high,
            &high_open,
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
        &high_open,
        &p.f.clock,
        &mut p.f.rng,
    )
    .unwrap();
    let (object, proof_plain) = protected_seal(&mut p.f.high, &p.high, &proof, &p.f.clock);
    let (_, low_open) = admit(&mut p.f.low, &p.low, object, &p.f.clock);
    assert_eq!(
        opened(
            &mut p.f.low,
            &p.low,
            &low_open,
            &proof_plain,
            &p.f.clock,
            &mut p.f.rng
        )
        .outcome,
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
    let (object, finish_plain) = protected_seal(&mut p.f.low, &p.low, &finish, &p.f.clock);
    let (_, high_finish) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    assert_eq!(
        opened(
            &mut p.f.high,
            &p.high,
            &high_finish,
            &finish_plain,
            &p.f.clock,
            &mut p.f.rng
        )
        .outcome,
        CryptoOutcome::Opened
    );
    assert!(p.f.low.established(p.f.id, &p.f.clock));
    assert!(p.f.high.established(p.f.id, &p.f.clock));

    // A different first result has its own closed rejection, retained after token/input removal.
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, mut bytes) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    bytes[0] ^= 1;
    let receipt = opened(
        &mut p.f.high,
        &p.high,
        &action,
        &bytes,
        &p.f.clock,
        &mut p.f.rng,
    );
    assert!(matches!(receipt.outcome, CryptoOutcome::Rejected(_)));
    assert!(action.input().is_err());
    p._runtimes[1].revoke_session();
    // End::new deliberately creates its unit call lease independently from the actor's child
    // registry. Model that original call ending too; actual Root fixtures use Directory's
    // actor-registered CallGrant and test the native session boundary separately.
    p.f.high.lease.revoke();
    let recovered = checked(
        &mut p.f.high,
        &p.high,
        (&action, &bytes),
        &p.f.clock,
        &mut p.f.rng,
        |_, _| panic!("history has no new effect"),
    )
    .unwrap();
    assert!(receipt.same_transition(&recovered));
    assert!(recovered.retired);
    bytes[0] ^= 1;
    assert_eq!(
        checked(
            &mut p.f.high,
            &p.high,
            (&action, &bytes),
            &p.f.clock,
            &mut p.f.rng,
            |_, _| panic!("changed output")
        )
        .err(),
        Some(VoiceError::RetryMismatch)
    );
}

#[test]
fn protected_result_local_refusal_retains_the_exact_token_and_original_attempt() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, bytes) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    action.admit_engine(action.engine_bytes()).unwrap();
    let original = p.f.high.join.confirmations.pairs[&p.f.id].attempt.0.clone();
    assert_eq!(
        checked(
            &mut p.f.high,
            &p.high,
            (&action, &bytes),
            &p.f.clock,
            &mut p.f.rng,
            |_, _| Err(VoiceError::Capacity)
        )
        .err(),
        Some(VoiceError::Capacity)
    );
    assert!(action.terminal_status().receipt.is_none());
    let pair = &p.f.high.join.confirmations.pairs[&p.f.id];
    assert!(pair.attempt.0.is_same(&original) && original.is_live());
    assert!(pair.low_nonce.is_none());
    let receipt = checked(
        &mut p.f.high,
        &p.high,
        (&action, &bytes),
        &p.f.clock,
        &mut p.f.rng,
        |_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Opened);
    assert!(!original.is_live());
    action.settle_engine();
}

#[test]
fn protected_result_rechecks_atomic_session_close_while_actual_policy_is_held() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, bytes) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    action.admit_engine(action.engine_bytes()).unwrap();
    let native_child = p.high.child_lease().unwrap();
    let closed = p.high.revocation_observer_for_test();
    let runtime = p._runtimes[1].clone();
    let mut revoker = None;
    let result = checked(
        &mut p.f.high,
        &p.high,
        (&action, &bytes),
        &p.f.clock,
        &mut p.f.rng,
        |_, _| {
            // This callback runs after final native samples with actual Group/Policy held. The
            // lock operation flips the actor/session gate, then blocks revoking this real child.
            revoker = Some(std::thread::spawn(move || runtime.revoke_session()));
            futures::executor::block_on(closed.wait_revoked());
            assert!(
                native_child.is_live(),
                "Policy still prevents child revocation"
            );
            Ok(())
        },
    );
    revoker.unwrap().join().unwrap();
    assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    assert!(!native_child.is_live());
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .low_nonce
        .is_none());
    action.settle_engine();
}

#[test]
fn protected_result_last_native_clock_cannot_publish_after_original_owner_close() {
    for owner in ["call", "pair", "chain", "actor"] {
        let mut p = fixture();
        let hello = start(&mut p, None);
        let (object, bytes) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
        let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
        action.admit_engine(action.engine_bytes()).unwrap();
        let pair = &p.f.high.join.confirmations.pairs[&p.f.id];
        let close = match owner {
            "call" => p.f.high.lease.clone(),
            "pair" => pair.pin.clone(),
            "chain" => pair.chain.as_ref().unwrap().lease(),
            _ => p.high.revocation_observer_for_test(),
        };
        let last = p.origin_clock.reads.load(Ordering::Acquire) + 2;
        *p.origin_clock.revoke.lock().unwrap() = Some((last, close));
        assert_eq!(
            checked(
                &mut p.f.high,
                &p.high,
                (&action, &bytes),
                &p.f.clock,
                &mut p.f.rng,
                |_, _| Ok(())
            )
            .err(),
            Some(VoiceError::Unauthorized),
            "{owner}"
        );
        assert!(action.terminal_status().receipt.is_none(), "{owner}");
        assert!(
            p.f.high.join.confirmations.pairs[&p.f.id]
                .low_nonce
                .is_none(),
            "{owner}"
        );
        action.settle_engine();
    }
}

#[derive(Debug)]
struct PreparationClock {
    inner: ManualClock,
    action: Arc<CryptoAction>,
    reads: AtomicUsize,
}

struct PreparationRng<'a> {
    inner: &'a mut ChaCha20Rng,
    action: &'a CryptoAction,
    called: bool,
}
impl rand_core::CryptoRng for PreparationRng<'_> {}
impl rand_core::RngCore for PreparationRng<'_> {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }
    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        self.called = true;
        assert!(
            self.action.result_fence_available_for_test(),
            "RNG underneath Engine"
        );
        self.action.settle_engine();
        assert!(
            !self.action.terminal_status().settled,
            "preparation has not drained"
        );
        assert!(self.action.input().is_ok());
        self.inner.fill_bytes(out);
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(out);
        Ok(())
    }
}

#[test]
fn protected_open_settlement_during_rng_keeps_custody_and_refuses_token_effects() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, bytes) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    let input = action.input().unwrap();
    let CryptoInput::DecryptConfirmation(packet) = input.as_input() else {
        panic!("open")
    };
    action.admit_engine(action.engine_bytes()).unwrap();
    let room = room(&p.f.high);
    let mut rng = PreparationRng {
        inner: &mut p.f.rng,
        action: &action,
        called: false,
    };
    let result = p.f.high.join.commit_protected_confirmation_result(
        ProtectedOpenedResult {
            actor: &p.high,
            room: &room,
            action: &action,
            plaintext: &bytes,
            claimed_tag: action.output_tag(&bytes).unwrap(),
        },
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        &mut rng,
        |_, _| panic!("settlement won before admission"),
    );
    assert!(rng.called);
    assert_eq!(result.err(), Some(VoiceError::Unauthorized));
    assert!(action.is_settled());
    assert!(action.terminal_status().receipt.is_none());
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .low_nonce
        .is_none());
    assert!(
        p.f.high
            .join
            .ledger
            .validate_inbound_commit(
                packet.token,
                packet.aad(),
                packet.header.sequence,
                packet.ciphertext(),
                &ca(&p.f.high.context, &p.f.high.lease),
                &p.f.clock
            )
            .is_ok(),
        "unpublished token remains untouched"
    );
}
impl Clock for PreparationClock {
    fn now_ms(&self) -> u64 {
        assert!(
            self.action.result_fence_available_for_test(),
            "packet housekeeping under Engine"
        );
        self.reads.fetch_add(1, Ordering::AcqRel);
        self.inner.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        assert!(
            self.action.result_fence_available_for_test(),
            "packet housekeeping under Engine"
        );
        self.reads.fetch_add(1, Ordering::AcqRel);
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
fn protected_result_packet_housekeeping_runs_before_the_result_fence() {
    let mut p = fixture();
    let hello = start(&mut p, None);
    let (object, bytes) = protected_seal(&mut p.f.low, &p.low, &hello, &p.f.clock);
    let (_, action) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    action.admit_engine(action.engine_bytes()).unwrap();
    let clock = PreparationClock {
        inner: p.f.clock.clone(),
        action: action.clone(),
        reads: AtomicUsize::new(0),
    };
    assert_eq!(
        checked(
            &mut p.f.high,
            &p.high,
            (&action, &bytes),
            &clock,
            &mut p.f.rng,
            |_, _| Ok(())
        )
        .unwrap()
        .outcome,
        CryptoOutcome::Opened
    );
    assert!(clock.reads.load(Ordering::Acquire) > 0);
    action.settle_engine();
}

#[test]
fn compatibility_derivation_also_prepares_housekeeping_before_engine() {
    let mut p = fixture();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let action =
        p.f.high
            .join
            .chain_derivation_action(
                p.f.id,
                &chain,
                &ca(&p.f.high.context, &p.f.high.lease),
                Arc::new(p.f.clock.clone()),
            )
            .unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let clock = PreparationClock {
        inner: p.f.clock.clone(),
        action: action.clone(),
        reads: AtomicUsize::new(0),
    };
    let receipt =
        p.f.high
            .join
            .commit_derivation_result(
                &action,
                action.output_tag(&[]).unwrap(),
                &ca(&p.f.high.context, &p.f.high.lease),
                &clock,
            )
            .unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    assert!(clock.reads.load(Ordering::Acquire) > 0);
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
}
