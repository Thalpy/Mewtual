//! Exact unpublished signatures are distinct from source-visible packet custody. Root's final
//! native fence is tested separately; these pin the lower affine preparation/publication seam.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingSigner<'a> {
    inner: &'a DeviceKeypair,
    calls: &'a AtomicUsize,
    revoke: Option<CallLease>,
}
impl VoiceSigner for CountingSigner<'_> {
    fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
        self.inner.voice_public_key()
    }
    fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let signature = self.inner.sign_voice(bytes)?;
        if let Some(lease) = &self.revoke {
            lease.revoke();
        }
        Ok(signature)
    }
}

#[test]
fn prepared_signed_stage_fits_the_original_outgoing_row_charge() {
    let mut f = fixture();
    let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    // Count the complete fixed object (including the stage discriminant) and its one retained
    // ciphertext body. The unchanged reserve formula pays header/body copies plus256 metadata.
    // Returned/preparing copies belong to their active caller/engine, not a second stored body.
    assert!(std::mem::size_of::<Outgoing>() + 32 <= 2 * (draft.aad.len() + 32) + 256);
}

#[test]
fn abandoned_signed_preparation_is_not_source_visible_and_exact_retry_never_signs_again() {
    let mut f = fixture();
    let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    let calls = AtomicUsize::new(0);
    let signer = CountingSigner {
        inner: &f.a.signer,
        calls: &calls,
        revoke: None,
    };
    let current = authority(&f.a.context, &f.a.lease);
    let before = f.resources.counts();
    let prepared =
        f.a.ledger
            .prepare_sealed_publication(draft.id, &[7; 32], &signer, &current, &f.clock)
            .unwrap();
    let inert = prepared.signed().clone();
    drop(prepared); // Models a proven local refusal: no final admission/publication happened.
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(f.resources.counts(), before);
    assert!(f.a.ledger.outgoing[&draft.id].sealed.published().is_none());
    assert_eq!(f.a.ledger.published_source_packets().count(), 0);
    assert!(f
        .a
        .ledger
        .source_authority(&inert, &current, Arc::new(f.clock.clone()))
        .is_err());
    assert_eq!(
        f.a.ledger
            .prepare_sealed_publication(draft.id, &[8; 32], &signer, &current, &f.clock)
            .err(),
        Some(VoiceError::RetryMismatch)
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    let prepared =
        f.a.ledger
            .prepare_sealed_publication(draft.id, &[7; 32], &signer, &current, &f.clock)
            .unwrap();
    assert_eq!(prepared.signed(), &inert);
    prepared
        .check_at(
            current.context.epoch,
            f.clock.now_ms(),
            f.clock.monotonic_ms(),
        )
        .unwrap();
    let published = prepared.publish();
    assert_eq!(published, inert);
    assert_eq!(
        f.a.ledger.published_source_packets().collect::<Vec<_>>(),
        vec![&published]
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(f
        .a
        .ledger
        .source_authority(&published, &current, Arc::new(f.clock.clone()))
        .is_ok());
    assert_eq!(
        f.a.ledger
            .submit(draft.id, &[7; 32], &signer, &current, &f.clock)
            .unwrap(),
        published
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
}

#[test]
fn final_signed_publication_rejects_original_owner_epoch_and_deadline_changes() {
    for change in 0..5 {
        let mut f = fixture();
        let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
        let pair = f.a.ledger.pairs[&HANDSHAKE].live.clone();
        let packet = f.a.ledger.outgoing[&draft.id].live.clone();
        let current = authority(&f.a.context, &f.a.lease);
        let prepared =
            f.a.ledger
                .prepare_sealed_publication(draft.id, &[7; 32], &f.a.signer, &current, &f.clock)
                .unwrap();
        let mut epoch = current.context.epoch;
        match change {
            0 => f.a.lease.revoke(),
            1 => pair.revoke(),
            2 => packet.revoke(),
            3 => epoch += 1,
            _ => {
                f.clock.advance_ms(CONSTRUCTION_MS);
            }
        }
        assert!(prepared
            .check_at(epoch, f.clock.now_ms(), f.clock.monotonic_ms())
            .is_err());
        drop(prepared);
        assert!(f.a.ledger.outgoing[&draft.id].sealed.published().is_none());
    }
}

#[test]
fn prepared_seal_first_publication_uses_source_and_exchange_deadlines_before_construction() {
    for exchange_bound in [false, true] {
        let mut f = fixture();
        let exchange = exchange_bound.then(|| f.a.prepare_sdp(&f.clock).unwrap());
        f.clock.advance_ms(14_000);
        let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
        // Isolate each original bound at this pure lower fence. The actual late pair/flight
        // construction is separately exercised through all three LocalJoin protected roles.
        if exchange_bound {
            f.a.ledger.sdp.as_mut().unwrap().retire = Some(16_000);
            f.a.ledger.outgoing.get_mut(&draft.id).unwrap().exchange = exchange;
        } else {
            f.a.ledger
                .outgoing
                .get_mut(&draft.id)
                .unwrap()
                .source_deadline = 16_000;
        }
        assert!(f.a.ledger.outgoing[&draft.id].construction > 17_000);
        let prepared =
            f.a.ledger
                .prepare_sealed_publication(
                    draft.id,
                    &[7; 32],
                    &f.a.signer,
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock,
                )
                .unwrap();
        f.clock.advance_ms(2_000);
        assert_eq!(
            prepared.check_at(f.a.context.epoch, f.clock.now_ms(), f.clock.monotonic_ms()),
            Err(VoiceError::Reservation)
        );
        drop(prepared);
        assert!(f.a.ledger.outgoing[&draft.id].sealed.published().is_none());
        assert!(f.a.ledger.outgoing[&draft.id].crypto.0.is_live());
    }
}

#[test]
fn signer_revocation_leaves_only_an_inert_signature_and_no_crypto_consumption() {
    let mut f = fixture();
    let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    let calls = AtomicUsize::new(0);
    let packet = f.a.ledger.outgoing[&draft.id].live.clone();
    let signer = CountingSigner {
        inner: &f.a.signer,
        calls: &calls,
        revoke: Some(packet),
    };
    assert_eq!(
        f.a.ledger
            .submit(
                draft.id,
                &[7; 32],
                &signer,
                &authority(&f.a.context, &f.a.lease),
                &f.clock
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(f.a.ledger.outgoing[&draft.id].sealed.published().is_none());
    assert!(f.a.ledger.outgoing[&draft.id].crypto.0.is_live());
}

#[test]
fn prepared_stage_prunes_without_returning_its_counter_or_stranding_credit() {
    let mut f = fixture();
    let before = f.resources.counts();
    let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    let prepared =
        f.a.ledger
            .prepare_sealed_publication(
                draft.id,
                &[7; 32],
                &f.a.signer,
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
            )
            .unwrap();
    drop(prepared);
    assert!(f.resources.counts().0 > before.0);
    f.clock.advance_ms(MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS);
    f.a.ledger.prune(&f.clock);
    assert!(f.a.ledger.outgoing.is_empty());
    assert_eq!(f.resources.counts(), before);
    let next = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    assert_eq!(next.sequence, draft.sequence + 1);
    assert_ne!(next.id, draft.id);
}

#[test]
fn final_prepared_stage_refuses_chain_or_exchange_retirement() {
    for exchange in [false, true] {
        let mut f = fixture();
        let draft = f.a.reserve(&f.clock, &mut f.rng).unwrap();
        let chain = CallLease::default();
        // Isolate the original owner gate. The exchange itself is an actual ledger-issued SDP
        // owner; the artificial association below makes no whole-chain readiness claim.
        let owner = exchange.then(|| f.a.prepare_sdp(&f.clock).unwrap());
        let outgoing = f.a.ledger.outgoing.get_mut(&draft.id).unwrap();
        outgoing.chain = Some(chain.clone());
        outgoing.exchange = owner.clone();
        let prepared =
            f.a.ledger
                .prepare_sealed_publication(
                    draft.id,
                    &[7; 32],
                    &f.a.signer,
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock,
                )
                .unwrap();
        if let Some(owner) = owner {
            owner.close();
        } else {
            chain.revoke();
        }
        assert_eq!(
            prepared.check_at(f.a.context.epoch, f.clock.now_ms(), f.clock.monotonic_ms()),
            Err(VoiceError::Unauthorized)
        );
        drop(prepared);
        assert!(f.a.ledger.outgoing[&draft.id].sealed.published().is_none());
    }
}
