//! The observation does not conflate local Finish signing, remote open or discovery lifetime.
use super::*;

fn observe(
    end: &mut End,
    actor: &ActorOwner,
    id: Nonce,
    clock: &ManualClock,
) -> Result<VoicePairConfirmation, VoiceError> {
    let room = room(end);
    let pair = &end.join.confirmations.pairs[&id];
    let selector = PairSelector {
        peer: pair.binding.peer,
        join: pair.binding.join,
        key_ref: pair.binding.key_ref,
        epoch: end.context.epoch,
    };
    end.join.protected_pair_confirmation(
        ProtectedPair {
            actor,
            room: &room,
            watch: &end.watch_lease,
            selector,
        },
        id,
        &ca(&end.context, &end.lease),
        clock,
        |_, _| Ok(()),
    )
}

#[test]
fn protected_pair_confirmation_is_asymmetric_and_survives_discovery_expiry_and_plan_retirement() {
    let mut p = fixture();
    assert_eq!(
        observe(&mut p.f.low, &p.low, p.f.id, &p.f.clock),
        Ok(VoicePairConfirmation::Confirming)
    );
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
    let low_control = finish.confirmation_control().unwrap();
    let (object, plain) = seal(&mut p.f.low, &finish, &p.f.clock);
    assert_eq!(
        observe(&mut p.f.low, &p.low, p.f.id, &p.f.clock),
        Ok(VoicePairConfirmation::Established)
    );
    assert_eq!(
        observe(&mut p.f.high, &p.high, p.f.id, &p.f.clock),
        Ok(VoicePairConfirmation::Confirming)
    );
    let (_, finish_open) = admit(&mut p.f.high, &p.high, object, &p.f.clock);
    let high_control = finish_open.confirmation_control().unwrap();
    open(
        &mut p.f.high,
        &finish_open,
        &plain,
        &p.f.clock,
        &mut p.f.rng,
    );
    // Observing cannot retire retry custody. The exact cached original Finish still exists.
    assert!(Arc::ptr_eq(
        &finish,
        &respond(
            &mut p.f.low,
            &p.low,
            p.f.id,
            &proof_open,
            &p.f.clock,
            &mut p.f.rng
        )
        .unwrap()
    ));
    // Retire each actual original confirmation plan explicitly, independently of the pair.
    // No drainage is inferred: retained action/input owners may still hold paid capacity.
    let _low_retirement = low_control.retire();
    let _high_retirement = high_control.retire();
    p.f.clock.advance_ms(60_001);
    p.f.low.join.collect_expired(&p.f.clock);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(p
        .f
        .low
        .watcher
        .peers(&wa(&p.f.low.context, &p.f.low.watch_lease), &p.f.clock)
        .unwrap()
        .is_empty());
    for (end, actor) in [(&mut p.f.low, &p.low), (&mut p.f.high, &p.high)] {
        let before = end.join.resources.counts();
        assert_eq!(
            observe(end, actor, p.f.id, &p.f.clock),
            Ok(VoicePairConfirmation::Established)
        );
        assert_eq!(end.join.resources.counts(), before);
    }
}
