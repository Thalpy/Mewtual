use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const HANDSHAKE: [u8; 32] = [9; 32];
fn member(_: DeviceId) -> bool {
    true
}
fn authority<'a>(context: &'a Context, lease: &'a CallLease) -> Authority<'a> {
    Authority {
        context,
        lease,
        member: &member,
    }
}
struct Endpoint {
    signer: DeviceKeypair,
    context: Context,
    lease: CallLease,
    join: Nonce,
    key_ref: [u8; 32],
    ledger: PacketLedger,
}
impl Endpoint {
    fn new(seed: u8, resources: Arc<Resources>) -> Self {
        let signer = DeviceKeypair::from_seed(&[seed; 32]);
        let lease = CallLease::default();
        let join = [seed; 32];
        let key_ref = [seed + 10; 32];
        let context = Context {
            scope: Scope::for_group(b"native exact group", Some([7; 32]), 12),
            local: signer.device_id(),
            now: 1000,
            epoch: 3,
            indirect: true,
        };
        let ledger = PacketLedger::new(&context, join, key_ref, lease.clone(), resources).unwrap();
        Self {
            signer,
            context,
            lease,
            join,
            key_ref,
            ledger,
        }
    }
    fn register(&mut self, peer: &Endpoint) {
        self.ledger
            .register_pair(
                PairBinding {
                    peer: peer.context.local,
                    join: peer.join,
                    key_ref: peer.key_ref,
                    handshake: HANDSHAKE,
                },
                &authority(&self.context, &self.lease),
            )
            .unwrap();
    }
    fn establish(&mut self) {
        self.ledger
            .establish(&HANDSHAKE, &authority(&self.context, &self.lease))
            .unwrap();
    }
    fn reserve(
        &mut self,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> Result<ReservedPacket, VoiceError> {
        self.ledger.reserve(
            self.signer.verifying_key().to_bytes(),
            ReserveRequest {
                handshake: HANDSHAKE,
                kind: EncryptedKind::Offer,
                offer: None,
                ciphertext_bytes: 32,
            },
            &authority(&self.context, &self.lease),
            clock,
            rng,
        )
    }
    fn submit(
        &mut self,
        packet: &ReservedPacket,
        ciphertext: &[u8],
        clock: &ManualClock,
    ) -> Result<SignedObject, VoiceError> {
        self.ledger.submit(
            packet.id,
            ciphertext,
            &self.signer,
            &authority(&self.context, &self.lease),
            clock,
        )
    }
    fn admit(
        &mut self,
        object: SignedObject,
        clock: &ManualClock,
    ) -> Result<InboundPacket, VoiceError> {
        self.ledger
            .admit_inbound(object, &authority(&self.context, &self.lease), clock)
    }
    fn commit(
        &mut self,
        packet: &InboundPacket,
        clock: &ManualClock,
    ) -> Result<CommittedPacket, VoiceError> {
        self.ledger.commit_inbound(
            packet.token,
            &packet.aad,
            packet.header.sequence,
            &packet.ciphertext,
            &authority(&self.context, &self.lease),
            clock,
        )
    }
}
struct Fixture {
    a: Endpoint,
    c: Endpoint,
    clock: ManualClock,
    rng: ChaCha20Rng,
    resources: Arc<Resources>,
}
fn fixture() -> Fixture {
    let resources = Arc::new(Resources::default());
    let mut a = Endpoint::new(1, resources.clone());
    let mut c = Endpoint::new(3, resources.clone());
    a.register(&c);
    c.register(&a);
    a.establish();
    c.establish();
    Fixture {
        a,
        c,
        clock: ManualClock::new(1000),
        rng: ChaCha20Rng::seed_from_u64(11),
        resources,
    }
}
fn send(f: &mut Fixture) -> (ReservedPacket, SignedObject, InboundPacket) {
    let reservation = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    let signed = f.a.submit(&reservation, &[42; 32], &f.clock).unwrap();
    let incoming = f.c.admit(signed.clone(), &f.clock).unwrap();
    (reservation, signed, incoming)
}

#[test]
fn prepared_ingress_origin_refusal_does_not_consume_replay_or_native_token() {
    let mut f = fixture();
    let reserved = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    let signed = f.a.submit(&reserved, &[42; 32], &f.clock).unwrap();
    let baseline = f.resources.counts();
    let prepared =
        f.c.ledger
            .prepare_inbound(
                signed.clone(),
                &authority(&f.c.context, &f.c.lease),
                &f.clock,
            )
            .unwrap();
    assert!(
        f.resources.counts().0 > baseline.0,
        "prepare owns all retained and event bytes"
    );
    assert_eq!(
        f.c.ledger
            .commit_prepared_inbound(
                prepared,
                &authority(&f.c.context, &f.c.lease),
                &f.clock,
                |_| Err(VoiceError::RateLimited)
            )
            .err(),
        Some(VoiceError::RateLimited)
    );
    assert_eq!(f.c.ledger.serial, 0);
    assert_eq!(f.c.ledger.pairs[&HANDSHAKE].replay.highest, 0);
    assert!(f.c.ledger.incoming.is_empty());
    assert_eq!(f.resources.counts(), baseline);
    let incoming = f.c.admit(signed.clone(), &f.clock).unwrap();
    assert_eq!(incoming.token, 1);
    assert_eq!(f.c.admit(signed, &f.clock).err(), Some(VoiceError::Replay));
    f.c.commit(&incoming, &f.clock).unwrap();
}

#[test]
fn actual_decrypt_event_keeps_byte_credit_after_token_retirement_and_receipt_drop() {
    for commit_first in [false, true] {
        let mut f = fixture();
        let (_, _, incoming) = send(&mut f);
        let charged = f.resources.counts();
        if commit_first {
            drop(f.c.commit(&incoming, &f.clock).unwrap());
        } else {
            f.c.ledger.retire_handshake(&HANDSHAKE);
        }
        assert!(f.c.ledger.incoming.is_empty());
        assert_eq!(
            f.resources.counts(),
            charged,
            "actual returned event still owns its payload charge"
        );
        drop(incoming);
        assert!(
            f.resources.counts().0 < charged.0,
            "only the last actual payload owner refunds bytes"
        );
    }
}

#[test]
fn collection_expires_cached_packet_bytes_but_preserves_decrypt_payload_replay_and_allocator() {
    let mut f = fixture();
    let (_, signed, incoming) = send(&mut f);
    let next = f.a.ledger.pairs[&HANDSHAKE].next;
    let replay = f.c.ledger.pairs[&HANDSHAKE].replay.clone();
    f.clock.advance_ms(20_001);
    f.a.ledger.collect_expired(&f.clock);
    f.c.ledger.collect_expired(&f.clock);
    assert!(f.a.ledger.outgoing.is_empty() && f.c.ledger.incoming.is_empty());
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, next);
    assert_eq!(f.c.ledger.pairs[&HANDSHAKE].replay.highest, replay.highest);
    assert_eq!(f.c.ledger.pairs[&HANDSHAKE].replay.seen, replay.seen);
    let held = f.resources.counts().0;
    assert!(
        held > 0,
        "actual returned decrypt bytes remain charged after cache expiry"
    );
    f.clock.set_wall_ms(1000);
    assert!(f.c.commit(&incoming, &f.clock).is_err());
    assert!(
        f.c.admit(signed, &f.clock).is_err(),
        "GC never resets the replay window"
    );
    drop(incoming);
    assert!(f.resources.counts().0 < held);
}

#[test]
fn prepared_ingress_serial_pair_and_end_fences_run_before_original_commit() {
    use std::cell::Cell;
    for scenario in 0..3 {
        let mut f = fixture();
        let reserved = f.a.reserve(&f.clock, &mut f.rng).unwrap();
        let signed = f.a.submit(&reserved, &[42; 32], &f.clock).unwrap();
        let prepared =
            f.c.ledger
                .prepare_inbound(
                    signed.clone(),
                    &authority(&f.c.context, &f.c.lease),
                    &f.clock,
                )
                .unwrap();
        let committed = Cell::new(false);
        if scenario == 0 {
            f.c.admit(signed, &f.clock).unwrap();
        }
        if scenario == 1 {
            f.c.ledger.retire_handshake(&HANDSHAKE);
        }
        let result = f.c.ledger.commit_prepared_inbound(
            prepared,
            &authority(&f.c.context, &f.c.lease),
            &f.clock,
            |admission| {
                // Model native End winning while the final Group/Policy transaction is blocked.
                // The actual endpoint callback repeats this check inside that final transaction.
                if scenario == 2 {
                    f.c.lease.revoke();
                }
                admission.check(&f.clock)?;
                committed.set(true);
                Ok(())
            },
        );
        assert!(result.is_err(), "scenario {scenario}");
        assert!(
            !committed.get(),
            "no rejected preparation spends original quota"
        );
        if scenario != 0 {
            assert_eq!(f.c.ledger.pairs[&HANDSHAKE].replay.highest, 0);
            assert!(f.c.ledger.incoming.is_empty());
            assert_eq!(f.c.ledger.serial, 0);
        }
    }
}

#[test]
fn a_future_issued_remote_packet_retains_both_clock_skew_windows() {
    let mut f = fixture();
    let sender_clock = ManualClock::new(6000);
    let reserved = f.a.reserve(&sender_clock, &mut f.rng).unwrap();
    let signed = f.a.submit(&reserved, &[42; 32], &sender_clock).unwrap();
    let incoming = f.c.admit(signed, &f.clock).unwrap();
    // Issued five seconds ahead of the observer: it remains valid until 26 seconds, including
    // post-expiry skew. A twenty-second local cap would incorrectly expire this admitted token.
    f.clock.advance_ms(21_000);
    let receipt = f.c.commit(&incoming, &f.clock).unwrap();
    f.c.ledger
        .receipt_current(&receipt, &authority(&f.c.context, &f.c.lease), &f.clock)
        .unwrap();
}

#[test]
fn native_draft_aad_and_sealed_signature_have_one_canonical_header() {
    let mut f = fixture();
    let (reservation, signed, incoming) = send(&mut f);
    assert_eq!(reservation.aad, signed.aad().unwrap());
    assert_eq!(incoming.aad, reservation.aad);
    assert!(SignedObject::decode(&signed.encode()).unwrap() == signed);
    let draft =
        f.a.ledger
            .outgoing
            .get(&reservation.id)
            .unwrap()
            .draft
            .clone();
    assert!(draft.seal(&f.c.signer, &[42; 32]).is_err());
    assert!(draft.seal(&f.a.signer, &[42; 15]).is_err());
    assert!(f.c.commit(&incoming, &f.clock).is_ok());
    assert_eq!(
        f.c.commit(&incoming, &f.clock).err(),
        Some(VoiceError::Reservation)
    );
}

#[test]
fn exact_submit_retries_cache_ciphertext_and_altered_retries_cannot_replace_it() {
    let mut f = fixture();
    let (reservation, signed, _) = send(&mut f);
    assert!(f.a.submit(&reservation, &[42; 32], &f.clock).unwrap() == signed);
    assert_eq!(
        f.a.submit(&reservation, &[43; 32], &f.clock).err(),
        Some(VoiceError::RetryMismatch)
    );
    assert!(f.a.submit(&reservation, &[42; 32], &f.clock).unwrap() == signed);
    f.clock.advance_ms(CONSTRUCTION_MS);
    assert!(
        f.a.submit(&reservation, &[42; 32], &f.clock).unwrap() == signed,
        "cached ciphertext does not reconstruct encryption after construction allowance"
    );
}

#[test]
fn failed_construction_cache_expiry_and_pair_reregistration_never_reset_a_nonce() {
    let mut f = fixture();
    let first = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    assert_eq!(first.sequence, 1);
    f.clock.advance_ms(CONSTRUCTION_MS);
    assert_eq!(
        f.a.submit(&first, &[42; 32], &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    f.a.register(&f.c);
    let second = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    assert_eq!(second.sequence, 2);
    f.clock.advance_ms(20_000);
    let third = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    assert_eq!(third.sequence, 3);
    assert_eq!(
        f.a.submit(&first, &[42; 32], &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(
        f.a.submit(&second, &[42; 32], &f.clock).err(),
        Some(VoiceError::Reservation)
    );
}

#[test]
fn inbound_tokens_bind_exact_aad_sequence_and_ciphertext_without_consuming_another_packet() {
    let mut f = fixture();
    let (_, _, first) = send(&mut f);
    let (_, _, second) = send(&mut f);
    for (aad, sequence, ciphertext) in [
        (&first.aad[..], first.header.sequence, &first.ciphertext[..]),
        (
            &second.aad[..],
            first.header.sequence,
            &second.ciphertext[..],
        ),
        (&second.aad[..], second.header.sequence, &[0u8; 32][..]),
    ] {
        assert_eq!(
            f.c.ledger
                .commit_inbound(
                    second.token,
                    aad,
                    sequence,
                    ciphertext,
                    &authority(&f.c.context, &f.c.lease),
                    &f.clock
                )
                .err(),
            Some(VoiceError::RetryMismatch)
        );
    }
    assert!(f.c.commit(&second, &f.clock).is_ok());
    assert!(f.c.commit(&first, &f.clock).is_ok());
}

#[test]
fn current_membership_exact_lease_scope_and_retirement_are_rechecked_after_decrypt() {
    let mut f = fixture();
    let (_, _, incoming) = send(&mut f);
    let wrong = CallLease::default();
    assert!(f
        .c
        .ledger
        .commit_inbound(
            incoming.token,
            &incoming.aad,
            incoming.header.sequence,
            &incoming.ciphertext,
            &authority(&f.c.context, &wrong),
            &f.clock
        )
        .is_err());
    let removed = |id| id != f.a.context.local;
    let denied = Authority {
        context: &f.c.context,
        lease: &f.c.lease,
        member: &removed,
    };
    assert!(f
        .c
        .ledger
        .commit_inbound(
            incoming.token,
            &incoming.aad,
            incoming.header.sequence,
            &incoming.ciphertext,
            &denied,
            &f.clock
        )
        .is_err());
    f.c.context.scope.room += 1;
    assert!(f.c.commit(&incoming, &f.clock).is_err());
    f.c.context.scope.room -= 1;
    f.c.ledger.retire_member(f.a.context.local);
    assert!(f.c.commit(&incoming, &f.clock).is_err());
    assert!(
        f.c.ledger
            .register_pair(
                PairBinding {
                    peer: f.a.context.local,
                    join: f.a.join,
                    key_ref: f.a.key_ref,
                    handshake: HANDSHAKE
                },
                &authority(&f.c.context, &f.c.lease)
            )
            .is_err(),
        "re-add cannot revive a retired join"
    );
    let (_, _, another) = send_to_fresh_receiver(&mut f);
    f.c.lease.revoke();
    assert!(f.c.commit(&another, &f.clock).is_err());
}

#[test]
fn opaque_committed_receipts_keep_exact_bytes_charged_and_never_revive_after_removal() {
    let mut f = fixture();
    let (_, object, incoming) = send(&mut f);
    let charged = f.resources.counts().0;
    let receipt = f.c.commit(&incoming, &f.clock).unwrap();
    assert_eq!(receipt.hash(), object.hash());
    assert_eq!(receipt.id(), object.id());
    assert_eq!(receipt.header(), &incoming.header);
    assert_eq!(f.resources.counts().0, charged);
    f.c.ledger
        .receipt_current(&receipt, &authority(&f.c.context, &f.c.lease), &f.clock)
        .unwrap();
    let absent = |id| id != f.a.context.local;
    assert!(f
        .c
        .ledger
        .receipt_current(
            &receipt,
            &Authority {
                context: &f.c.context,
                lease: &f.c.lease,
                member: &absent
            },
            &f.clock
        )
        .is_err());
    assert!(f
        .c
        .ledger
        .receipt_current(&receipt, &authority(&f.c.context, &f.c.lease), &f.clock)
        .is_err());
    drop(f.a);
    drop(f.c);
    assert!(f.resources.counts().0 > 0);
    drop(receipt);
    assert!(
        f.resources.counts().0 > 0,
        "the separately returned decrypt event is still an actual byte owner"
    );
    drop(incoming);
    assert_eq!(f.resources.counts(), (0, 0));
}
fn send_to_fresh_receiver(f: &mut Fixture) -> (ReservedPacket, SignedObject, InboundPacket) {
    f.c = Endpoint::new(3, f.resources.clone());
    f.c.register(&f.a);
    f.c.establish();
    send(f)
}

#[test]
fn helper_or_connection_changes_cannot_redeliver_an_admitted_packet() {
    let mut f = fixture();
    let (_, object, incoming) = send(&mut f);
    assert_eq!(
        f.c.admit(object.clone(), &f.clock).err(),
        Some(VoiceError::Replay)
    );
    f.c.commit(&incoming, &f.clock).unwrap();
    assert_eq!(f.c.admit(object, &f.clock).err(), Some(VoiceError::Replay));
}

#[test]
fn replay_window_reorders_unique_counters_but_rejects_duplicates_and_pruned_old_counters() {
    let mut replay = Replay::default();
    replay.admit(1).unwrap();
    replay.admit(130).unwrap();
    assert_eq!(replay.admit(1), Err(VoiceError::Replay));
    assert_eq!(replay.admit(2), Err(VoiceError::Replay));
    replay.admit(3).unwrap();
    assert_eq!(replay.admit(3), Err(VoiceError::Replay));
    replay.admit(129).unwrap();
    assert_eq!(replay.admit(130), Err(VoiceError::Replay));
    assert_eq!(replay.admit(0), Err(VoiceError::Replay));
    replay.admit(u64::MAX).unwrap();
    assert_eq!(replay.admit(130), Err(VoiceError::Replay));
}

#[test]
fn correctly_signed_wrong_join_key_reference_handshake_and_target_refuse() {
    let mut f = fixture();
    let reservation = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    let object = f.a.submit(&reservation, &[42; 32], &f.clock).unwrap();
    for dimension in 0..6 {
        let Body::Encrypted {
            mut header,
            ciphertext,
        } = object.body.clone()
        else {
            panic!()
        };
        match dimension {
            0 => header.source_join[0] ^= 1,
            1 => header.target_join[0] ^= 1,
            2 => header.source_key_ref[0] ^= 1,
            3 => header.target_key_ref[0] ^= 1,
            4 => header.handshake[0] ^= 1,
            _ => {
                header.target = DeviceKeypair::from_seed(&[4; 32]).device_id();
                header.low_to_high = object.issuer < header.target;
            }
        }
        let wrong = SignedObject::issue(
            object.scope,
            &f.a.signer,
            object.id,
            object.issued,
            object.expires,
            Body::Encrypted { header, ciphertext },
        )
        .unwrap();
        assert!(
            f.c.admit(wrong, &f.clock).is_err(),
            "dimension {dimension} accepted"
        );
    }
    assert!(
        f.c.admit(object, &f.clock).is_ok(),
        "invalid scope does not burn a valid sequence"
    );
}

#[test]
fn pending_completed_and_inbound_records_have_hard_caps_without_resetting_counters() {
    let mut f = fixture();
    for _ in 0..RESERVATIONS {
        send(&mut f);
    }
    assert_eq!(
        f.a.reserve(&f.clock, &mut f.rng).err(),
        Some(VoiceError::Capacity)
    );
    let mut overflow = f.c.ledger.incoming.values().next().unwrap().object.clone();
    overflow.id = [91; 32];
    let Body::Encrypted { header, .. } = &mut overflow.body else {
        panic!()
    };
    header.sequence = 33;
    header.offer = Some(OfferId {
        issuer: f.a.context.local,
        message: overflow.id,
    });
    let overflow = SignedObject::issue(
        overflow.scope,
        &f.a.signer,
        overflow.id,
        overflow.issued,
        overflow.expires,
        overflow.body,
    )
    .unwrap();
    assert_eq!(
        f.c.admit(overflow.clone(), &f.clock).err(),
        Some(VoiceError::Capacity)
    );
    let token = *f.c.ledger.incoming.keys().next().unwrap();
    let stored = f.c.ledger.incoming.get(&token).unwrap().object.clone();
    let Body::Encrypted { header, ciphertext } = &stored.body else {
        panic!()
    };
    f.c.ledger
        .commit_inbound(
            token,
            &stored.aad().unwrap(),
            header.sequence,
            ciphertext,
            &authority(&f.c.context, &f.c.lease),
            &f.clock,
        )
        .unwrap();
    assert!(
        f.c.admit(overflow, &f.clock).is_ok(),
        "capacity refusal did not burn unadmitted replay state"
    );
    f.clock.advance_ms(20_000);
    assert_eq!(f.a.reserve(&f.clock, &mut f.rng).unwrap().sequence, 33);
}

#[test]
fn monotonic_expiry_survives_wall_clock_rollback_and_keeps_nonce_history() {
    let mut f = fixture();
    let (reservation, _, incoming) = send(&mut f);
    f.clock.set_wall_ms(0);
    f.clock.advance_ms(20_000);
    assert_eq!(
        f.a.submit(&reservation, &[42; 32], &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(
        f.c.commit(&incoming, &f.clock).err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(f.a.reserve(&f.clock, &mut f.rng).unwrap().sequence, 2);
}

#[test]
fn nonce_and_pair_capacity_cannot_wrap_or_be_freed_by_retirement() {
    let mut f = fixture();
    f.a.ledger.pairs.get_mut(&HANDSHAKE).unwrap().next = u64::MAX;
    assert_eq!(
        f.a.reserve(&f.clock, &mut f.rng).err(),
        Some(VoiceError::Reservation)
    );
    for index in 10..25 {
        f.a.ledger
            .register_pair(
                PairBinding {
                    peer: f.c.context.local,
                    join: [index; 32],
                    key_ref: f.c.key_ref,
                    handshake: [index; 32],
                },
                &authority(&f.a.context, &f.a.lease),
            )
            .unwrap();
    }
    f.a.ledger.retire_member(f.c.context.local);
    assert_eq!(
        f.a.ledger.register_pair(
            PairBinding {
                peer: f.c.context.local,
                join: [99; 32],
                key_ref: f.c.key_ref,
                handshake: [99; 32]
            },
            &authority(&f.a.context, &f.a.lease)
        ),
        Err(VoiceError::Capacity)
    );
}

#[test]
fn engine_and_packet_cache_share_credits_and_refusal_does_not_allocate_a_nonce() {
    let mut f = fixture();
    let full = f
        .resources
        .charge(f.a.context.scope.group, 256 * 1024, ResourceKind::Retained)
        .unwrap();
    assert_eq!(
        f.a.reserve(&f.clock, &mut f.rng).err(),
        Some(VoiceError::Capacity)
    );
    drop(full);
    assert_eq!(f.a.reserve(&f.clock, &mut f.rng).unwrap().sequence, 1);
    assert!(f.resources.counts().0 > 0);
    drop(f.a);
    assert_eq!(f.resources.counts(), (0, 0));
}

#[test]
fn application_packets_wait_for_native_confirmation_and_wrong_confirmation_role_refuses() {
    let mut f = fixture();
    f.a.ledger.pairs.get_mut(&HANDSHAKE).unwrap().status = Status::Confirming;
    assert!(f.a.reserve(&f.clock, &mut f.rng).is_err());
    let role = if f.a.context.local < f.c.context.local {
        EncryptedKind::ConfirmHello
    } else {
        EncryptedKind::ConfirmProof
    };
    let reserve = ReserveRequest {
        handshake: HANDSHAKE,
        kind: role,
        offer: None,
        ciphertext_bytes: 32,
    };
    assert!(f
        .a
        .ledger
        .reserve(
            f.a.signer.verifying_key().to_bytes(),
            reserve,
            &authority(&f.a.context, &f.a.lease),
            &f.clock,
            &mut f.rng
        )
        .is_ok());
    let wrong = if role == EncryptedKind::ConfirmHello {
        EncryptedKind::ConfirmProof
    } else {
        EncryptedKind::ConfirmHello
    };
    let reserve = ReserveRequest {
        handshake: HANDSHAKE,
        kind: wrong,
        offer: None,
        ciphertext_bytes: 32,
    };
    assert!(f
        .a
        .ledger
        .reserve(
            f.a.signer.verifying_key().to_bytes(),
            reserve,
            &authority(&f.a.context, &f.a.lease),
            &f.clock,
            &mut f.rng
        )
        .is_err());
}

#[test]
fn cloned_leases_cannot_construct_or_reconstruct_an_allocator_for_the_same_live_join() {
    let f = fixture();
    assert!(PacketLedger::new(
        &f.a.context,
        f.a.join,
        f.a.key_ref,
        f.a.lease.clone(),
        f.resources.clone()
    )
    .is_err());
    assert!(
        f.a.lease.is_live(),
        "a refused duplicate does not retire its existing allocator"
    );
    let lease = f.a.lease.clone();
    let context = Context {
        scope: f.a.context.scope,
        local: f.a.context.local,
        now: 1000,
        epoch: 3,
        indirect: true,
    };
    let join = f.a.join;
    let key_ref = f.a.key_ref;
    drop(f.a);
    assert!(!lease.is_live());
    assert!(PacketLedger::new(&context, join, key_ref, lease, f.resources).is_err());
}

#[test]
fn concurrent_allocator_claims_have_exactly_one_owner() {
    let lease = CallLease::default();
    let resources = Arc::new(Resources::default());
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let jobs = (0..2)
        .map(|_| {
            let lease = lease.clone();
            let resources = resources.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let signer = DeviceKeypair::from_seed(&[1; 32]);
                let context = Context {
                    scope: Scope::for_group(b"group", None, 1),
                    local: signer.device_id(),
                    now: 1000,
                    epoch: 0,
                    indirect: false,
                };
                barrier.wait();
                PacketLedger::new(&context, [1; 32], [2; 32], lease, resources)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = jobs
        .into_iter()
        .map(|job| job.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert!(lease.is_live());
    drop(outcomes);
    assert!(!lease.is_live());
}

#[test]
fn final_submit_and_delivery_repeat_future_time_checks_without_consuming_valid_tokens() {
    let mut f = fixture();
    f.clock.set_wall_ms(10_000);
    let (reservation, signed, incoming) = send(&mut f);
    let pending = f.a.reserve(&f.clock, &mut f.rng).unwrap();
    f.clock.set_wall_ms(0);
    assert_eq!(
        f.a.submit(&reservation, &[42; 32], &f.clock).err(),
        Some(VoiceError::Freshness)
    );
    assert_eq!(
        f.a.submit(&pending, &[42; 32], &f.clock).err(),
        Some(VoiceError::Freshness)
    );
    assert_eq!(
        f.c.commit(&incoming, &f.clock).err(),
        Some(VoiceError::Freshness)
    );
    f.clock.set_wall_ms(10_000);
    assert!(f.a.submit(&reservation, &[42; 32], &f.clock).unwrap() == signed);
    assert!(f.a.submit(&pending, &[42; 32], &f.clock).is_ok());
    assert!(f.c.commit(&incoming, &f.clock).is_ok());
}
