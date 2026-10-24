//! Packet preparation rollback, isolated from the native producer's final action/history transaction.
use super::*;

fn ice(a: &Endpoint) -> ReserveRequest {
    ReserveRequest {
        handshake: HANDSHAKE,
        kind: EncryptedKind::Ice,
        offer: Some(OfferId {
            issuer: a.context.local,
            message: [19; 32],
        }),
        ciphertext_bytes: 32,
    }
}

#[test]
fn affine_outgoing_drop_before_or_after_validation_changes_no_sequence_or_row() {
    let mut f = fixture();
    for validated in [false, true] {
        let baseline = f.resources.counts();
        let prepared =
            f.a.ledger
                .prepare_outgoing(
                    f.a.signer.verifying_key().to_bytes(),
                    ice(&f.a),
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock,
                    &mut f.rng,
                )
                .unwrap();
        assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
        assert!(f.a.ledger.outgoing.is_empty());
        assert!(f.resources.counts().0 > baseline.0);
        if validated {
            let (publication, reservation) =
                f.a.ledger
                    .validate_prepared_outgoing(
                        prepared,
                        &authority(&f.a.context, &f.a.lease),
                        &f.clock,
                    )
                    .unwrap();
            assert_eq!(reservation.sequence, 1);
            drop((publication, reservation));
        } else {
            drop(prepared);
        }
        assert_eq!(f.resources.counts(), baseline);
        assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
        assert!(f.a.ledger.outgoing.is_empty());
    }
    assert_eq!(f.a.reserve(&f.clock, &mut f.rng).unwrap().sequence, 1);
}

#[test]
fn affine_outgoing_competing_preparation_cannot_publish_a_reused_sequence() {
    let mut f = fixture();
    let first =
        f.a.ledger
            .prepare_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                ice(&f.a),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
    let second =
        f.a.ledger
            .prepare_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                ice(&f.a),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
    let (publication, reservation) =
        f.a.ledger
            .validate_prepared_outgoing(first, &authority(&f.a.context, &f.a.lease), &f.clock)
            .unwrap();
    publication.publish();
    assert_eq!(
        f.a.ledger
            .validate_prepared_outgoing(second, &authority(&f.a.context, &f.a.lease), &f.clock)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.a.ledger.outgoing.len(), 1);
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 1);
    assert!(f.a.ledger.outgoing.contains_key(&reservation.id));
    assert_eq!(f.a.reserve(&f.clock, &mut f.rng).unwrap().sequence, 2);
}

#[test]
fn affine_outgoing_abandoned_offer_does_not_pin_offer_role_or_start_work_window() {
    let mut f = fixture();
    f.a.prepare_sdp(&f.clock).unwrap();
    let request = || ReserveRequest {
        handshake: HANDSHAKE,
        kind: EncryptedKind::Offer,
        offer: None,
        ciphertext_bytes: MAX_SDP_PLAINTEXT + 16,
    };
    let baseline = f.resources.counts();
    let prepared =
        f.a.ledger
            .prepare_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                request(),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
    let sdp = f.a.ledger.sdp.as_ref().unwrap();
    assert!(sdp.role == SdpRole::Ready && sdp.local.is_none() && sdp.retire.is_none());
    drop(prepared);
    assert_eq!(f.resources.counts(), baseline);
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
    let original_mono = f.clock.monotonic_ms();
    let original_wall = f.clock.now_ms();
    let prepared =
        f.a.ledger
            .prepare_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                request(),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
    f.clock.advance_ms(3000);
    let (publication, reservation) =
        f.a.ledger
            .validate_prepared_outgoing(prepared, &authority(&f.a.context, &f.a.lease), &f.clock)
            .unwrap();
    publication.publish();
    let sdp = f.a.ledger.sdp.as_ref().unwrap();
    assert!(
        sdp.role
            == SdpRole::Initiator(OfferId {
                issuer: f.a.context.local,
                message: reservation.id
            })
    );
    assert_eq!(sdp.local, Some(reservation.id));
    assert_eq!(
        sdp.retire,
        Some(original_mono + MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS)
    );
    assert_eq!(
        reservation.expires,
        original_wall + MAX_PACKET_LIFETIME_MS,
        "delayed publication never renews original time bounds"
    );
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 1);
}

#[test]
fn affine_outgoing_refuses_epoch_replacement_and_original_construction_expiry() {
    for expired in [false, true] {
        let mut f = fixture();
        let baseline = f.resources.counts();
        let prepared =
            f.a.ledger
                .prepare_outgoing(
                    f.a.signer.verifying_key().to_bytes(),
                    ice(&f.a),
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock,
                    &mut f.rng,
                )
                .unwrap();
        if expired {
            f.clock.advance_ms(CONSTRUCTION_MS);
        } else {
            f.a.context.epoch += 1;
        }
        assert_eq!(
            f.a.ledger
                .validate_prepared_outgoing(
                    prepared,
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock
                )
                .err(),
            Some(if expired {
                VoiceError::Reservation
            } else {
                VoiceError::Unauthorized
            })
        );
        assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
        assert!(f.a.ledger.outgoing.is_empty());
        assert_eq!(f.resources.counts(), baseline);
    }
}

#[test]
fn affine_outgoing_equal_serialized_call_and_pool_cannot_adopt_another_native_lease() {
    let mut f = fixture();
    let mut replacement = Endpoint::new(1, f.resources.clone());
    replacement.register(&f.c);
    replacement.establish();
    let baseline = f.resources.counts();
    let prepared =
        f.a.ledger
            .prepare_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                ice(&f.a),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
    assert_eq!(
        replacement
            .ledger
            .validate_prepared_outgoing(
                prepared,
                &authority(&replacement.context, &replacement.lease),
                &f.clock
            )
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
    assert_eq!(replacement.ledger.pairs[&HANDSHAKE].next, 0);
    assert!(f.a.ledger.outgoing.is_empty() && replacement.ledger.outgoing.is_empty());
    assert_eq!(f.resources.counts(), baseline);
}

#[test]
fn ordinary_outgoing_final_clock_end_does_not_publish_a_dead_reservation() {
    for revoke_at in [2, 3, 4] {
        let mut f = fixture();
        let baseline = f.resources.counts();
        let clock = RevokingClock {
            inner: f.clock.clone(),
            lease: f.a.lease.clone(),
            reads: std::sync::atomic::AtomicUsize::new(0),
            revoke_at,
        };
        assert_eq!(
            f.a.ledger
                .reserve(
                    f.a.signer.verifying_key().to_bytes(),
                    ice(&f.a),
                    &authority(&f.a.context, &f.a.lease),
                    &clock,
                    &mut f.rng
                )
                .err(),
            Some(VoiceError::Unauthorized)
        );
        assert!(f.a.ledger.outgoing.is_empty());
        assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
        assert_eq!(f.resources.counts(), baseline);
    }
}

/// Packet-level resource composition supplies an actual native binding directly, as the
/// neighboring allocator tests do. Fresh presence/LocalJoin producer checks are a separate gate.
fn confirmation_fixture() -> (Fixture, Arc<crate::voice_v2::chain::PreparedChain>) {
    let resources = Arc::new(Resources::default());
    let mut a = Endpoint::new(1, resources.clone());
    let mut c = Endpoint::new(3, resources.clone());
    a.register(&c);
    c.register(&a);
    if a.context.local > c.context.local {
        std::mem::swap(&mut a, &mut c);
    }
    let clock = ManualClock::new(1000);
    let pair = &a.ledger.pairs[&HANDSHAKE];
    let binding = crate::voice_v2::chain::Binding {
        scope: a.context.scope,
        epoch: a.context.epoch,
        local: a.context.local,
        local_join: a.join,
        local_key: a.key_ref,
        handshake: HANDSHAKE,
        peer: pair.binding.peer,
        peer_join: pair.binding.join,
        peer_key: pair.binding.key_ref,
        generation: 1,
        call: a.lease.clone(),
        pair: pair.live.clone(),
        deadline: clock.monotonic_ms() + 15_000,
        low: true,
    };
    let chain = crate::voice_v2::chain::PreparedChain::new(
        resources.clone(),
        binding,
        a.ledger.action_slots(),
        a.ledger.packet_slots(),
    )
    .unwrap();
    (
        Fixture {
            a,
            c,
            clock,
            rng: ChaCha20Rng::seed_from_u64(161),
            resources,
        },
        chain,
    )
}
fn hello_request() -> ReserveRequest {
    ReserveRequest {
        handshake: HANDSHAKE,
        kind: EncryptedKind::ConfirmHello,
        offer: None,
        ciphertext_bytes: 32,
    }
}

#[test]
fn protected_output_slot_refuses_wrong_native_owners_role_and_table_without_spending_claims() {
    let (f, chain) = confirmation_fixture();
    let workflow = chain.workflow().unwrap();
    let pair = &f.a.ledger.pairs[&HANDSHAKE];
    let header = EncryptedHeader {
        source_join: f.a.join,
        target: pair.binding.peer,
        target_join: pair.binding.join,
        source_key_ref: f.a.key_ref,
        target_key_ref: pair.binding.key_ref,
        handshake: HANDSHAKE,
        low_to_high: true,
        kind: EncryptedKind::ConfirmHello,
        sequence: 1,
        offer: None,
    };
    for wrong in ["resources", "call", "pair", "table", "epoch", "role", "key"] {
        let other_resources = Arc::new(Resources::default());
        let other_lease = CallLease::default();
        let other_slots = PacketSlots::new();
        let mut context = Context {
            scope: f.a.context.scope,
            local: f.a.context.local,
            now: f.a.context.now,
            epoch: f.a.context.epoch,
            indirect: f.a.context.indirect,
        };
        let mut proposed = header.clone();
        if wrong == "epoch" {
            context.epoch += 1;
        }
        if wrong == "role" {
            proposed.kind = EncryptedKind::ConfirmProof;
        }
        if wrong == "key" {
            proposed.target_key_ref = [233; 32];
        }
        assert!(
            workflow
                .outgoing_confirmation_slot(
                    if wrong == "resources" {
                        &other_resources
                    } else {
                        &f.resources
                    },
                    &context,
                    if wrong == "call" {
                        &other_lease
                    } else {
                        &f.a.lease
                    },
                    if wrong == "pair" {
                        &other_lease
                    } else {
                        &pair.live
                    },
                    if wrong == "table" {
                        &other_slots
                    } else {
                        f.a.ledger.packet_slots()
                    },
                    &proposed
                )
                .is_err(),
            "{wrong}"
        );
    }
    let mut held = Vec::new();
    for _ in 0..6 {
        held.push(
            workflow
                .outgoing_confirmation_slot(
                    &f.resources,
                    &f.a.context,
                    &f.a.lease,
                    &pair.live,
                    f.a.ledger.packet_slots(),
                    &header,
                )
                .unwrap(),
        );
    }
    assert_eq!(
        workflow
            .outgoing_confirmation_slot(
                &f.resources,
                &f.a.context,
                &f.a.lease,
                &pair.live,
                f.a.ledger.packet_slots(),
                &header
            )
            .err(),
        Some(VoiceError::Capacity)
    );
    drop(held);
    assert!(workflow
        .outgoing_confirmation_slot(
            &f.resources,
            &f.a.context,
            &f.a.lease,
            &pair.live,
            f.a.ledger.packet_slots(),
            &header
        )
        .is_ok());
}

#[test]
fn protected_output_preparation_near_original_pair_deadline_cannot_publish_after_it() {
    let (mut f, chain) = confirmation_fixture();
    f.clock.advance_ms(MAX_PACKET_LIFETIME_MS - 1);
    let prepared =
        f.a.ledger
            .prepare_protected_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                hello_request(),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
                chain.workflow().unwrap(),
            )
            .unwrap();
    f.clock.advance_ms(1);
    assert_eq!(
        f.a.ledger
            .validate_prepared_outgoing(prepared, &authority(&f.a.context, &f.a.lease), &f.clock)
            .err(),
        Some(VoiceError::Reservation)
    );
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
    assert!(f.a.ledger.outgoing.is_empty());
}

#[derive(Debug)]
struct LastWallRevocation {
    inner: ManualClock,
    target: CallLease,
}
impl Clock for LastWallRevocation {
    fn now_ms(&self) -> u64 {
        self.target.revoke();
        self.inner.now_ms()
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
fn protected_source_observes_each_owner_revoked_on_the_last_clock_callback_immediately() {
    use catcoms_rt::RequestAuthority;
    for owner in ["call", "pair", "packet", "chain"] {
        let (mut f, chain) = confirmation_fixture();
        let prepared =
            f.a.ledger
                .prepare_protected_outgoing(
                    f.a.signer.verifying_key().to_bytes(),
                    hello_request(),
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock,
                    &mut f.rng,
                    chain.workflow().unwrap(),
                )
                .unwrap();
        let (publication, reservation) = f
            .a
            .ledger
            .validate_prepared_outgoing(prepared, &authority(&f.a.context, &f.a.lease), &f.clock)
            .unwrap();
        publication.publish();
        let signed =
            f.a.ledger
                .submit(
                    reservation.id,
                    &[22; 32],
                    &f.a.signer,
                    &authority(&f.a.context, &f.a.lease),
                    &f.clock,
                )
                .unwrap();
        let mut source =
            f.a.ledger
                .source_authority(
                    &signed,
                    &authority(&f.a.context, &f.a.lease),
                    Arc::new(f.clock.clone()),
                )
                .unwrap();
        let target = match owner {
            "call" => source.local.clone(),
            "pair" => source.pair.clone(),
            "packet" => source.packet.clone(),
            "chain" => source.chain.clone().unwrap(),
            _ => unreachable!(),
        };
        source.clock = Arc::new(LastWallRevocation {
            inner: f.clock.clone(),
            target,
        });
        assert!(
            !source.authorized(),
            "{owner} closes during this same observation"
        );
    }
}

#[test]
fn affine_protected_outgoing_uses_actual_reserved_packet_and_bytes_under_ordinary_pressure() {
    let (mut f, chain) = confirmation_fixture();
    let other = Endpoint::new(7, Arc::new(Resources::default()));
    let other_handshake = [10; 32];
    f.a.ledger
        .register_pair(
            PairBinding {
                peer: other.context.local,
                join: other.join,
                key_ref: other.key_ref,
                handshake: other_handshake,
            },
            &authority(&f.a.context, &f.a.lease),
        )
        .unwrap();
    let ordinary_kind = if f.a.context.local < other.context.local {
        EncryptedKind::ConfirmHello
    } else {
        EncryptedKind::ConfirmProof
    };
    // Fill the shared table through actual ordinary packet APIs for another pair. Six
    // low-side output positions remain reserved; an ordinary packet cannot take those claims.
    for _ in 0..RESERVATIONS - 6 {
        f.a.ledger
            .reserve(
                f.a.signer.verifying_key().to_bytes(),
                ReserveRequest {
                    handshake: other_handshake,
                    kind: ordinary_kind,
                    offer: None,
                    ciphertext_bytes: 32,
                },
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
            )
            .unwrap();
    }
    assert_eq!(
        f.a.ledger
            .reserve(
                f.a.signer.verifying_key().to_bytes(),
                ReserveRequest {
                    handshake: other_handshake,
                    kind: ordinary_kind,
                    offer: None,
                    ciphertext_bytes: 32,
                },
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng
            )
            .err(),
        Some(VoiceError::Capacity)
    );
    let baseline = f.resources.counts().0;
    let full = f
        .resources
        .charge(
            f.a.context.scope.group,
            crate::voice_v2::resources::GROUP_BYTES - baseline,
            ResourceKind::Retained,
        )
        .unwrap();
    let prepared =
        f.a.ledger
            .prepare_protected_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                hello_request(),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
                chain.workflow().unwrap(),
            )
            .unwrap();
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
    let (publication, reservation) =
        f.a.ledger
            .validate_prepared_outgoing(prepared, &authority(&f.a.context, &f.a.lease), &f.clock)
            .unwrap();
    assert_eq!(reservation.sequence, 1);
    publication.publish();
    assert_eq!(f.a.ledger.outgoing.len(), RESERVATIONS - 5);
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 1);
    assert_eq!(
        f.a.ledger.pairs[&other_handshake].next,
        (RESERVATIONS - 6) as u64
    );
    drop(full);
}

#[test]
fn affine_protected_outgoing_close_after_prepare_refuses_publication_without_returning_actual_credit_early(
) {
    let (mut f, chain) = confirmation_fixture();
    let prepared =
        f.a.ledger
            .prepare_protected_outgoing(
                f.a.signer.verifying_key().to_bytes(),
                hello_request(),
                &authority(&f.a.context, &f.a.lease),
                &f.clock,
                &mut f.rng,
                chain.workflow().unwrap(),
            )
            .unwrap();
    chain.close();
    let held = f.resources.counts().0;
    assert!(
        held > 2048,
        "actual prepared packet owns variable credit after close"
    );
    assert_eq!(
        f.a.ledger
            .validate_prepared_outgoing(prepared, &authority(&f.a.context, &f.a.lease), &f.clock)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(
        f.resources.counts().0 < held,
        "rejected local preparation retires its actual owner"
    );
    assert_eq!(f.a.ledger.pairs[&HANDSHAKE].next, 0);
    assert!(f.a.ledger.outgoing.is_empty());
}
