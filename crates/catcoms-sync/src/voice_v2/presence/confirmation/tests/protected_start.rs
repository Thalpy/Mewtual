//! Actual protected low producer: native packets, candidate history, actions and local-origin
//! transaction. Trusted AEAD output is mocked; these tests make no IPC or live RTC claim.
use super::*;
use crate::voice_v2::chain::PreparedChain;
use crate::voice_v2::crypto_actions::CryptoInput;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::{ActorOwner, Runtime, View};
use rand_core::RngCore;

fn low_fixture() -> (Fixture, Arc<Runtime>, ActorOwner, Arc<PreparedChain>) {
    let clock = ManualClock::new(1000);
    let runtime = Runtime::new(Arc::new(clock.clone()));
    let mut f = fixture_with_resources(clock, runtime.resources());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(f.low.context.scope.group, f.low.context.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: f.low.context.epoch,
            members: [f.low.context.local, f.high.context.local].into(),
            policy: f.low.context.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let chain = f
        .low
        .join
        .prepare_crypto_chain(f.id, &ca(&f.low.context, &f.low.lease), &f.clock)
        .unwrap();
    (f, runtime, actor, chain)
}

fn start(
    f: &mut Fixture,
    actor: &ActorOwner,
    predecessor: Option<Nonce>,
) -> Result<Arc<CryptoAction>, VoiceError> {
    start_checked(f, actor, predecessor, |_, _| Ok(()))
}

fn start_checked(
    f: &mut Fixture,
    actor: &ActorOwner,
    predecessor: Option<Nonce>,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<CryptoAction>, VoiceError> {
    let room = RoomAuthority {
        scope: f.low.context.scope,
        lease: f.low.lease.clone(),
        custody: None, // Root tests must supply the actual directory custody.
    };
    f.low.join.start_protected_hello(
        ProtectedStart {
            actor,
            room: &room,
            handshake: f.id,
            predecessor,
        },
        &ca(&f.low.context, &f.low.lease),
        Arc::new(f.clock.clone()),
        &mut f.rng,
        final_owner,
    )
}

fn tuple(action: &CryptoAction) -> (Nonce, u64, u64, Vec<u8>) {
    let input = action.input().unwrap();
    let CryptoInput::EncryptConfirmation(flight) = input.as_input() else {
        panic!("native seal")
    };
    (
        flight.reservation.id,
        flight.action,
        flight.reservation.sequence,
        flight.plaintext.clone(),
    )
}

struct RepeatedNonceRng<'a> {
    first: Nonce,
    remaining: &'a mut ChaCha20Rng,
    reads: usize,
}
impl RngCore for RepeatedNonceRng<'_> {
    fn next_u32(&mut self) -> u32 {
        self.remaining.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.remaining.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.reads += 1;
        if self.reads == 1 {
            assert_eq!(bytes.len(), 32);
            bytes.copy_from_slice(&self.first);
        } else {
            self.remaining.fill_bytes(bytes);
        }
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(bytes);
        Ok(())
    }
}
impl rand_core::CryptoRng for RepeatedNonceRng<'_> {}

#[test]
fn protected_low_nonce_cannot_cycle_a_b_a_or_spend_the_third_candidate_on_entropy_reuse() {
    let (mut f, _runtime, actor, _chain) = low_fixture();
    let first = start(&mut f, &actor, None).unwrap();
    let first_nonce = f.low.join.confirmations.pairs[&f.id].low_nonce.unwrap();
    let second = start(&mut f, &actor, Some(tuple(&first).0)).unwrap();
    let predecessor = Some(tuple(&second).0);
    let before = f.pool.counts();
    let current_nonce = f.low.join.confirmations.pairs[&f.id].low_nonce;
    let room = RoomAuthority {
        scope: f.low.context.scope,
        lease: f.low.lease.clone(),
        custody: None,
    };
    let mut rng = RepeatedNonceRng {
        first: first_nonce,
        remaining: &mut f.rng,
        reads: 0,
    };
    assert_eq!(
        f.low
            .join
            .start_protected_hello(
                ProtectedStart {
                    actor: &actor,
                    room: &room,
                    handshake: f.id,
                    predecessor
                },
                &ca(&f.low.context, &f.low.lease),
                Arc::new(f.clock.clone()),
                &mut rng,
                |_, _| Ok(())
            )
            .err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(
        rng.reads, 1,
        "refusal precedes packet identity/row/action preparation"
    );
    let pair = &f.low.join.confirmations.pairs[&f.id];
    assert_eq!(pair.starts, 2);
    assert_eq!(pair.action_serial, 2);
    assert_eq!(pair.low_nonce, current_nonce);
    assert_eq!(f.pool.counts(), before);
    let third = start(&mut f, &actor, predecessor).unwrap();
    assert_eq!(tuple(&third).2, 3);
}

#[test]
fn protected_low_start_lost_reply_recovers_exact_action_before_and_after_terminal_compaction() {
    let (mut f, _runtime, actor, _chain) = low_fixture();
    let action = start(&mut f, &actor, None).unwrap();
    let original = tuple(&action);
    let before = f.pool.counts();
    let mut expected_rng = f.rng.clone();
    for _ in 0..5 {
        assert!(Arc::ptr_eq(&action, &start(&mut f, &actor, None).unwrap()));
        assert_eq!(tuple(&action), original);
    }
    assert_eq!(f.pool.counts(), before);
    assert_eq!(f.rng.clone().next_u64(), expected_rng.next_u64());
    action.admit_engine(action.engine_bytes()).unwrap();
    let ciphertext = vec![22; original.3.len() + 16];
    let result = f
        .low
        .join
        .submit_confirmation_result(
            &action,
            &ciphertext,
            action.output_tag(&ciphertext).unwrap(),
            &f.low.signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    assert!(matches!(result.outcome, CryptoOutcome::Sealed { .. }));
    action.settle_engine();
    action.acknowledge_result(&result).unwrap();
    assert!(action.input().is_err());
    assert!(Arc::ptr_eq(&action, &start(&mut f, &actor, None).unwrap()));
    assert_eq!(f.low.join.confirmations.pairs[&f.id].starts, 1);
}

#[test]
fn protected_low_predecessors_never_branch_or_become_fresh_after_all_three_candidates() {
    let (mut f, _runtime, actor, _chain) = low_fixture();
    let mut predecessor = None;
    let mut issued = Vec::new();
    for sequence in 1..=3 {
        let action = start(&mut f, &actor, predecessor).unwrap();
        let identity = tuple(&action);
        assert_eq!(identity.2, sequence);
        issued.push((predecessor, action.clone()));
        predecessor = Some(identity.0);
    }
    let before = f.pool.counts();
    let mut expected_rng = f.rng.clone();
    for (predecessor, original) in &issued {
        assert!(Arc::ptr_eq(
            original,
            &start(&mut f, &actor, *predecessor).unwrap()
        ));
    }
    assert_eq!(
        start(&mut f, &actor, predecessor).err(),
        Some(VoiceError::RateLimited)
    );
    assert_eq!(
        start(&mut f, &actor, Some([231; 32])).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(f.rng.clone().next_u64(), expected_rng.next_u64());
    assert_eq!(f.pool.counts(), before);
    assert_eq!(f.low.join.confirmations.pairs[&f.id].starts, 3);
    assert_eq!(f.low.join.confirmations.pairs[&f.id].action_serial, 3);
}

#[test]
fn protected_low_final_refusal_publishes_no_candidate_packet_or_pair_nonce() {
    let (mut f, _runtime, actor, _chain) = low_fixture();
    let before = f.pool.counts();
    assert_eq!(
        start_checked(&mut f, &actor, None, |_, _| Err(VoiceError::Capacity)).err(),
        Some(VoiceError::Capacity)
    );
    let pair = &f.low.join.confirmations.pairs[&f.id];
    assert_eq!(pair.starts, 0);
    assert_eq!(pair.action_serial, 0);
    assert!(pair.action.is_none() && pair.low_nonce.is_none());
    assert_eq!(f.pool.counts(), before);
    let action = start(&mut f, &actor, None).unwrap();
    assert_eq!(tuple(&action).2, 1);
    assert_eq!(f.low.join.confirmations.pairs[&f.id].starts, 1);
}

#[test]
fn protected_low_running_old_seal_cannot_sign_after_replacement_or_chain_close() {
    for close in [false, true] {
        let (mut f, _runtime, actor, chain) = low_fixture();
        let action = start(&mut f, &actor, None).unwrap();
        let identity = tuple(&action);
        action.admit_engine(action.engine_bytes()).unwrap();
        let next = if close {
            chain.close();
            None
        } else {
            let next = start(&mut f, &actor, Some(identity.0)).unwrap();
            assert_eq!(tuple(&next).2, 2);
            assert_eq!(
                next.admit_engine(next.engine_bytes()).err(),
                Some(VoiceError::Capacity),
                "the old actual engine still occupies the protected lane"
            );
            Some(next)
        };
        assert_eq!(f.pool.counts().1, 1, "actual old engine remains charged");
        let ciphertext = vec![22; identity.3.len() + 16];
        assert!(f
            .low
            .join
            .submit_confirmation_result(
                &action,
                &ciphertext,
                action.output_tag(&ciphertext).unwrap(),
                &f.low.signer,
                &ca(&f.low.context, &f.low.lease),
                &f.clock
            )
            .is_err());
        assert!(action.terminal_status().receipt.is_none());
        assert!(action.admit_engine(action.engine_bytes()).is_err());
        action.settle_engine();
        if let Some(next) = next {
            // The open Workflow retains its prepaid lane after actual loan settlement. Prove
            // reuse through the real next engine, rather than expecting an ordinary-pool refund.
            assert_eq!(f.pool.counts().1, 1);
            next.admit_engine(next.engine_bytes()).unwrap();
            next.settle_engine();
            chain.close();
        }
        assert_eq!(f.pool.counts().1, 0);
    }
}

#[test]
fn protected_low_supersession_retires_unstarted_progress_but_not_a_held_input_or_fixed_history() {
    for closed in [false, true] {
        let (mut f, _runtime, actor, chain) = low_fixture();
        let first = start(&mut f, &actor, None).unwrap();
        let id = tuple(&first).0;
        let held = first.input().unwrap();
        let _next = start(&mut f, &actor, Some(id)).unwrap();
        assert!(first.is_settled());
        assert!(first.input().is_err());
        assert!(first.admit_engine(first.engine_bytes()).is_err());
        let CryptoInput::EncryptConfirmation(flight) = held.as_input() else {
            panic!("retained input")
        };
        let payload_bytes = flight.reservation.aad.len() + flight.plaintext.len() + 16;
        assert!(Arc::ptr_eq(&first, &start(&mut f, &actor, None).unwrap()));
        if closed {
            chain.close();
        }
        let before = f.pool.counts().0;
        drop(held);
        assert_eq!(f.pool.counts().0, if closed { before - payload_bytes } else { before },
            "an open reserve retains returned capacity; a closed reserve releases the last actual payload");
    }
}

#[test]
fn protected_low_source_custody_closes_with_chain_even_after_seal_settlement() {
    let (mut f, _runtime, actor, chain) = low_fixture();
    let action = start(&mut f, &actor, None).unwrap();
    let identity = tuple(&action);
    let ciphertext = vec![22; identity.3.len() + 16];
    action.admit_engine(action.engine_bytes()).unwrap();
    let signed = f
        .low
        .join
        .submit_confirmation_action(
            &action,
            &ciphertext,
            &f.low.signer,
            &ca(&f.low.context, &f.low.lease),
            &f.clock,
        )
        .unwrap();
    action.settle_engine();
    let source = f
        .low
        .join
        .source_packet_activity(
            &signed,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone()),
        )
        .unwrap();
    assert!(source.check(&signed).is_ok());
    chain.close();
    assert!(source.check(&signed).is_err());
    assert!(f
        .low
        .join
        .source_packet_activity(
            &signed,
            &ca(&f.low.context, &f.low.lease),
            Arc::new(f.clock.clone())
        )
        .is_err());
}

#[test]
fn protected_low_start_and_engine_progress_use_reserved_capacity_with_full_ordinary_bytes() {
    let (mut f, _runtime, actor, chain) = low_fixture();
    let before = f.pool.counts().0;
    let full = f
        .pool
        .charge(
            f.low.context.scope.group,
            256 * 1024 - before,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let action = start(&mut f, &actor, None).unwrap();
    assert_eq!(tuple(&action).2, 1);
    action.admit_engine(action.engine_bytes()).unwrap();
    let held = action.input().unwrap();
    chain.close();
    assert_eq!(f.pool.counts().1, 1);
    action.settle_engine();
    assert_eq!(f.pool.counts().1, 0);
    assert!(matches!(
        held.as_input(),
        CryptoInput::EncryptConfirmation(_)
    ));
    drop((held, full));
}

#[test]
fn protected_low_original_pair_deadline_is_not_renewed_by_a_late_candidate_or_epoch_change() {
    for change_epoch in [false, true] {
        let (mut f, _runtime, actor, _chain) = low_fixture();
        let first = start(&mut f, &actor, None).unwrap();
        let predecessor = Some(tuple(&first).0);
        if change_epoch {
            f.low.context.epoch += 1;
        } else {
            f.clock.advance_ms(MAX_PACKET_LIFETIME_MS);
        }
        assert!(start(&mut f, &actor, predecessor).is_err());
        assert!(start(&mut f, &actor, None).is_err());
        assert_eq!(f.low.join.confirmations.pairs[&f.id].starts, 1);
    }
}
