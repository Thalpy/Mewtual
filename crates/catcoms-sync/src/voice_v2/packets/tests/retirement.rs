use super::*;

#[test]
fn consumed_incoming_retains_its_table_slot_until_the_actual_outer_retirement() {
    let mut f = fixture();
    let incoming: Vec<_> = (0..RESERVATIONS).map(|_| send(&mut f).2).collect();
    let first = &incoming[0];
    let prepared =
        f.c.ledger
            .validate_inbound_commit(
                first.token,
                first.aad(),
                first.header.sequence,
                first.ciphertext(),
                &authority(&f.c.context, &f.c.lease),
                &f.clock,
            )
            .unwrap();
    let (packet, remainder) = f.c.ledger.consume_validated_inbound_deferred(prepared);
    assert_eq!(f.c.ledger.incoming.len(), RESERVATIONS - 1);
    assert_eq!(
        f.c.ledger.packet_slots.incoming.charge(1).err(),
        Some(VoiceError::Capacity)
    );
    drop(packet); // The signed-body owner alone does not release the row's separate slot.
    assert_eq!(
        f.c.ledger.packet_slots.incoming.charge(1).err(),
        Some(VoiceError::Capacity)
    );
    drop(remainder);
    assert!(f.c.ledger.packet_slots.incoming.charge(1).is_ok());
}

#[test]
fn prepared_incoming_final_observation_rechecks_each_original_owner_without_consumption() {
    for change in 0..7 {
        let mut f = fixture();
        let (_, _, incoming) = send(&mut f);
        let pending = CallLease::default();
        let pair = f.c.ledger.pairs[&HANDSHAKE].live.clone();
        let token = f.c.ledger.incoming[&incoming.token].token.0.clone();
        // Isolate the private pending gate; production confirmation factories select this from
        // actual role/sequence state, never a received descriptor. This fixture uses an ICE row.
        f.c.ledger
            .incoming
            .get_mut(&incoming.token)
            .unwrap()
            .confirmation = Some(PendingConfirmation::Attempt(pending.clone()));
        let exchange = (change == 6).then(|| f.c.prepare_sdp(&f.clock).unwrap());
        f.c.ledger
            .incoming
            .get_mut(&incoming.token)
            .unwrap()
            .exchange = exchange.clone();
        let prepared =
            f.c.ledger
                .validate_inbound_commit(
                    incoming.token,
                    incoming.aad(),
                    incoming.header.sequence,
                    incoming.ciphertext(),
                    &authority(&f.c.context, &f.c.lease),
                    &f.clock,
                )
                .unwrap();
        let mut epoch = f.c.context.epoch;
        match change {
            0 => f.c.lease.revoke(),
            1 => pair.revoke(),
            2 => token.revoke(),
            3 => pending.revoke(),
            4 => epoch += 1,
            5 => {
                f.clock.advance_ms(MAX_PACKET_LIFETIME_MS + CLOCK_SKEW_MS);
            }
            _ => exchange.unwrap().close(),
        }
        assert!(prepared
            .check_at(epoch, f.clock.now_ms(), f.clock.monotonic_ms())
            .is_err());
        assert!(
            f.c.ledger.incoming.contains_key(&incoming.token),
            "refusal must not consume"
        );
    }
}
