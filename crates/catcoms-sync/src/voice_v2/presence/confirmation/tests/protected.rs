//! Actual LocalJoin/packet/action/origin composition. Browser AEAD is the same trusted-output
//! seam as the neighboring tests; these do not claim Root namespace, IPC or audible acceptance.
use super::*;
use crate::voice_v2::chain::{ConfirmationAdmission, PreparedChain};
use crate::voice_v2::runtime::{ActorOwner, Runtime, View};

fn protected_fixture() -> (Fixture, Arc<Runtime>, ActorOwner, Arc<PreparedChain>) {
    let clock = ManualClock::new(1000);
    let runtime = Runtime::new(Arc::new(clock.clone()));
    let mut f = fixture_with_resources(clock, runtime.resources());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(f.high.context.scope.group, f.high.context.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: f.high.context.epoch,
            members: [f.low.context.local, f.high.context.local].into(),
            policy: f.high.context.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let chain = f
        .high
        .join
        .prepare_crypto_chain(f.id, &ca(&f.high.context, &f.high.lease), &f.clock)
        .unwrap();
    (f, runtime, actor, chain)
}

fn admit(
    f: &mut Fixture,
    actor: &ActorOwner,
    object: SignedObject,
) -> Result<Arc<ConfirmationAdmission>, VoiceError> {
    let envelope = Envelope::Direct(object);
    let ticket = actor.admit_transport_verification(f.low.context.local, &envelope.encode())?;
    admit_verified(f, actor, &ticket, &envelope)
}

fn admit_verified(
    f: &mut Fixture,
    actor: &ActorOwner,
    ticket: &crate::voice_v2::runtime::IngressTicket,
    envelope: &Envelope,
) -> Result<Arc<ConfirmationAdmission>, VoiceError> {
    let parent = f.high.lease.clone();
    let room = crate::voice_v2::helper::RoomAuthority {
        scope: f.high.context.scope,
        lease: parent.clone(),
        custody: None, // Unit confirmation fixture; Root tests use the real directory owner.
    };
    f.high.join.admit_protected_confirmation(
        actor,
        ticket,
        envelope,
        &room,
        &ca(&f.high.context, &f.high.lease),
        Arc::new(f.clock.clone()),
        |_, _| {
            if parent.is_live() {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        },
    )
}

#[test]
fn protected_hello_uses_three_spent_candidate_lanes_even_when_no_nonce_was_accepted() {
    let (mut f, _runtime, actor, _chain) = protected_fixture();
    let mut saved = Vec::new();
    for candidate in 0..3 {
        let (mut flight, object) = hello(&mut f);
        let admission = admit(&mut f, &actor, object.clone()).unwrap();
        assert_eq!(admission.candidate(), candidate);
        let actions = f
            .high
            .join
            .pull_crypto_actions(&ca(&f.high.context, &f.high.lease))
            .unwrap();
        // Pending pulls exclude terminal actions; ACK plus settlement compacts their input.
        // Actual action/history rows remain retained; no old candidate becomes available.
        assert_eq!(actions.len(), 1);
        let action = actions.iter().find(|action| {
            action.input().is_ok_and(|input| matches!(input.as_input(), CryptoInput::DecryptConfirmation(packet) if packet.token == admission.token))
        }).unwrap().clone();
        action.admit_engine(action.engine_bytes()).unwrap();
        flight.plaintext[0] ^= 1;
        let result = f
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
        assert!(matches!(result.outcome, CryptoOutcome::Rejected(_)));
        action.settle_engine();
        action.acknowledge_result(&result).unwrap();
        saved.push((object, admission, action));
    }
    assert_eq!(f.high.join.confirmations.pairs[&f.id].starts, 0);
    assert!(f.high.join.confirmations.pairs[&f.id].low_nonce.is_none());
    for (object, receipt, _) in &saved {
        assert!(Arc::ptr_eq(
            receipt,
            &admit(&mut f, &actor, object.clone()).unwrap()
        ));
    }
    // A valid member signature does not gain a fourth candidate by changing identity/sequence.
    // AEAD is intentionally opaque at admission, so this is meaningful before decryption.
    let mut body = saved[2].0.body().clone();
    let Body::Encrypted { header, .. } = &mut body else {
        panic!("encrypted Hello")
    };
    header.sequence += 1;
    let fourth = SignedObject::issue(
        f.low.context.scope,
        &f.low.signer,
        [219; 32],
        1000,
        16_000,
        body,
    )
    .unwrap();
    let before = f.pool.counts();
    assert_eq!(
        admit(&mut f, &actor, fourth).err(),
        Some(VoiceError::RateLimited)
    );
    assert_eq!(f.pool.counts(), before);
    assert!(f
        .high
        .join
        .pull_crypto_actions(&ca(&f.high.context, &f.high.lease))
        .unwrap()
        .is_empty());
    assert_eq!(saved.len(), 3);
    assert!(saved
        .iter()
        .all(|(_, _, action)| action.terminal_status().receipt.is_some()));
}

#[test]
fn protected_hello_consumer_and_engine_use_reserved_custody_under_full_ordinary_pool() {
    let (mut f, _runtime, actor, chain) = protected_fixture();
    let (_, object) = hello(&mut f);
    let envelope = Envelope::Direct(object);
    // This tests the consumer boundary: transport verification already owns its separately
    // charged ticket. It does not establish whole wire/IPC progress under ordinary pressure.
    let ticket = actor
        .admit_transport_verification(f.low.context.local, &envelope.encode())
        .unwrap();
    let before = f.pool.counts().0;
    let full = f
        .pool
        .charge(
            f.high.context.scope.group,
            256 * 1024 - before,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let admission = admit_verified(&mut f, &actor, &ticket, &envelope).unwrap();
    let action = action(&mut f);
    action.admit_engine(action.engine_bytes()).unwrap();
    let input = action.input().unwrap();
    chain.close();
    assert!(!admission.authorized());
    assert!(action.with_pending(|| Ok(())).is_err());
    assert!(
        action.input().is_ok(),
        "running actual engine retains its input"
    );
    action.settle_engine();
    assert!(
        action.input().is_err(),
        "no result means no first effect after settlement"
    );
    assert!(
        matches!(input.as_input(), CryptoInput::DecryptConfirmation(_)),
        "held input remains owned after action compaction"
    );
    drop((input, full, ticket));
    // Historical admission has no engine/input custody; retain the receipt while observing
    // that the original reserve and actual engine/input owners have drained.
    assert!(admission.retains_identity());
    assert_eq!(f.pool.counts().1, 0);
    assert!(f.pool.counts().0 < before);
}

fn action(f: &mut Fixture) -> Arc<CryptoAction> {
    let actions = f
        .high
        .join
        .pull_crypto_actions(&ca(&f.high.context, &f.high.lease))
        .unwrap();
    assert_eq!(actions.len(), 1);
    actions[0].clone()
}

#[test]
fn protected_high_rejects_distinct_hello_ids_repeating_accepted_nonce_without_replacing_progress() {
    // Both immediate A -> A and A -> B -> A are possible malicious-peer inputs. The trusted
    // AEAD seam supplies repeated valid canonical plaintext under distinct authenticated IDs.
    for count in [2u8, 3] {
        let (mut f, _runtime, actor, _chain) = protected_fixture();
        let mut first_plaintext = None;
        let mut saved = None;
        for index in 0..count {
            let (flight, object) = hello(&mut f);
            let bytes = if index == count - 1 {
                first_plaintext.clone().unwrap()
            } else {
                first_plaintext.get_or_insert_with(|| flight.plaintext.clone());
                flight.plaintext
            };
            admit(&mut f, &actor, object).unwrap();
            let action = action(&mut f);
            action.admit_engine(action.engine_bytes()).unwrap();
            let result = f
                .high
                .join
                .commit_confirmation_result(
                    &action,
                    &bytes,
                    action.output_tag(&bytes).unwrap(),
                    &ca(&f.high.context, &f.high.lease),
                    &f.clock,
                    &mut f.rng,
                )
                .unwrap();
            let pair = &f.high.join.confirmations.pairs[&f.id];
            let current = (
                pair.starts,
                pair.action_serial,
                pair.low_nonce,
                pair.high_nonce,
            );
            if index == count - 1 {
                assert!(matches!(
                    result.outcome,
                    CryptoOutcome::Rejected(VoiceError::Replay)
                ));
                assert_eq!(Some(current), saved);
                assert!(
                    pair.hello.candidate(u64::from(count)).is_ok(),
                    "rejected nonce reuse cannot advance accepted-Hello floor"
                );
            } else {
                assert!(matches!(result.outcome, CryptoOutcome::Opened));
                saved = Some(current);
            }
            action.settle_engine();
            action.acknowledge_result(&result).unwrap();
            let recovered = f
                .high
                .join
                .commit_confirmation_result(
                    &action,
                    &bytes,
                    action.output_tag(&bytes).unwrap(),
                    &ca(&f.high.context, &f.high.lease),
                    &f.clock,
                    &mut f.rng,
                )
                .unwrap();
            assert!(result.same_transition(&recovered));
        }
        let pair = &f.high.join.confirmations.pairs[&f.id];
        assert_eq!(
            pair.protected_hello_tags.iter().flatten().count(),
            count as usize - 1
        );
    }
}

#[test]
fn protected_hello_atomically_publishes_action_and_exact_admission_without_decoding() {
    let (mut f, _runtime, actor, _chain) = protected_fixture();
    let (_, object) = hello(&mut f);
    let receipt = admit(&mut f, &actor, object.clone()).unwrap();
    let action = action(&mut f);
    assert_eq!(receipt.candidate(), 0);
    assert_eq!(f.high.join.confirmations.pairs[&f.id].starts, 0);
    assert!(f.high.join.confirmations.pairs[&f.id].low_nonce.is_none());
    let input = action.input().unwrap();
    let CryptoInput::DecryptConfirmation(packet) = input.as_input() else {
        panic!("decrypt input");
    };
    assert_eq!(packet.token, receipt.token);
    assert_eq!(packet.object_hash(), object.hash());
    let before = f.pool.counts();
    let again = admit(&mut f, &actor, object).unwrap();
    assert!(Arc::ptr_eq(&receipt, &again));
    assert_eq!(f.pool.counts(), before);
    assert!(Arc::ptr_eq(&action, &super::protected::action(&mut f)));
}

#[test]
fn protected_hello_origin_refusal_leaves_no_token_candidate_or_action_and_retries() {
    let (mut f, _runtime, actor, _chain) = protected_fixture();
    let (_, object) = hello(&mut f);
    actor
        .with_limits(|limits| {
            for _ in 0..40 {
                limits.origin(
                    f.low.context.local,
                    crate::voice_v2::limits::Traffic::Setup,
                    f.clock.monotonic_ms(),
                )?;
            }
            Ok(())
        })
        .unwrap();
    let before = f.pool.counts();
    assert_eq!(
        admit(&mut f, &actor, object.clone()).err(),
        Some(VoiceError::RateLimited)
    );
    assert_eq!(f.pool.counts(), before);
    assert!(f
        .high
        .join
        .pull_crypto_actions(&ca(&f.high.context, &f.high.lease))
        .unwrap()
        .is_empty());
    f.clock.advance_ms(1000);
    let admitted = admit(&mut f, &actor, object).unwrap();
    assert_eq!(admitted.candidate(), 0);
    assert_eq!(
        admitted.token, 1,
        "refusal cannot publish token serial or replay"
    );
    assert_eq!(
        action(&mut f).operation(),
        CryptoOperation::DecryptConfirmation
    );
}

#[test]
fn protected_hello_first_result_rejects_epoch_change_before_token_or_nonce_effect() {
    let (mut f, _runtime, actor, _chain) = protected_fixture();
    let (flight, object) = hello(&mut f);
    admit(&mut f, &actor, object).unwrap();
    let action = action(&mut f);
    action.admit_engine(action.engine_bytes()).unwrap();
    f.high.context.epoch += 1;
    assert_eq!(
        f.high.join.commit_confirmation_result(
            &action,
            &flight.plaintext,
            action.output_tag(&flight.plaintext).unwrap(),
            &ca(&f.high.context, &f.high.lease),
            &f.clock,
            &mut f.rng
        ),
        Err(VoiceError::Unauthorized)
    );
    assert!(action.terminal_status().receipt.is_none());
    assert_eq!(f.high.join.confirmations.pairs[&f.id].starts, 0);
    assert!(f.high.join.confirmations.pairs[&f.id].low_nonce.is_none());
    action.settle_engine();
}

#[test]
fn protected_hello_accepted_and_rejected_results_survive_epoch_change_and_lost_replies() {
    for malformed in [false, true] {
        let (mut f, _runtime, actor, _chain) = protected_fixture();
        let (mut flight, object) = hello(&mut f);
        let admission = admit(&mut f, &actor, object.clone()).unwrap();
        let action = action(&mut f);
        action.admit_engine(action.engine_bytes()).unwrap();
        if malformed {
            flight.plaintext[0] ^= 1;
        }
        let tag = action.output_tag(&flight.plaintext).unwrap();
        let result = f
            .high
            .join
            .commit_confirmation_result(
                &action,
                &flight.plaintext,
                tag,
                &ca(&f.high.context, &f.high.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
        assert_eq!(
            matches!(result.outcome, CryptoOutcome::Rejected(_)),
            malformed
        );
        action.settle_engine();
        action.acknowledge_result(&result).unwrap();
        // Token consumption and the Hello floor do not erase the independent admission ACK.
        assert!(Arc::ptr_eq(
            &admission,
            &admit(&mut f, &actor, object).unwrap()
        ));
        let pair = &f.high.join.confirmations.pairs[&f.id];
        let saved = (
            pair.starts,
            pair.action_serial,
            pair.low_nonce,
            pair.high_nonce,
        );
        let before = f.pool.counts();
        f.high.context.epoch += 1;
        let recovered = f
            .high
            .join
            .commit_confirmation_result(
                &action,
                &flight.plaintext,
                tag,
                &ca(&f.high.context, &f.high.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
        assert!(recovered.same_transition(&result));
        let pair = &f.high.join.confirmations.pairs[&f.id];
        assert_eq!(
            (
                pair.starts,
                pair.action_serial,
                pair.low_nonce,
                pair.high_nonce
            ),
            saved
        );
        assert_eq!(f.pool.counts(), before);
        assert!(action.admit_engine(action.engine_bytes()).is_err());
    }
}
