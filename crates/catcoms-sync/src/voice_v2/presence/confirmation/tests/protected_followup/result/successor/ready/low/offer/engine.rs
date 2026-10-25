//! Maximum LOW Seal under ordinary pressure uses its original prepaid Exchange.
//! This is Root admission only, not actual native/browser crypto or media evidence.
use super::*;
use crate::voice_v2::crypto_actions::SdpEngineRead;
use crate::voice_v2::sdp::SdpEngineAdmission;

#[test]
fn sdp_engine_maximum_seal_uses_original_exchange_without_native_engine_sized_copy() {
    let (mut p, phase, intent, _) = fixture();
    let mut namespace = ReceiptNamespace::default();
    let action = offer(
        &mut p,
        &intent,
        &mut namespace,
        &vec![b'a'; MAX_SDP_PLAINTEXT],
        |_, _| Ok(()),
    )
    .unwrap();
    let input = action.input().unwrap();
    let CryptoInput::SealSdp(flight) = input.as_input() else {
        panic!("original Seal")
    };
    assert_eq!(flight.plaintext.len(), MAX_SDP_PLAINTEXT);
    let resources = p._runtimes[0].resources();
    let group = p.f.low.context.scope.group;
    let pressure = resources
        .charge(
            group,
            crate::voice_v2::resources::GROUP_BYTES - resources.group_counts(group).unwrap().0,
            crate::voice_v2::resources::ResourceKind::Retained,
        )
        .unwrap();
    let prepared = action
        .prepare_sdp_engine_candidate(&intent, action.engine_bytes())
        .unwrap();
    let room = room(&p.f.low);
    let watch = crate::voice_v2::WatchLease::new(p.f.low.lease.clone());
    let current = p.f.low.join.prepare_sdp_input_current(
        &p.low,
        &room,
        &action,
        &intent,
        &p.f.low.context,
        &p.f.low.lease,
        &watch,
    );
    let admitted = prepared
        .with_observation(|observed| {
            let _native = NativeFenceProbe::new(&p.origin_clock);
            let step = SdpEngineAdmission::new(observed, current);
            assert!(step.belongs_to_action(&action));
            step.commit()
        })
        .unwrap();
    assert!(matches!(admitted, SdpEngineRead::Permit(serial) if serial == action.serial()));
    action.retire_if_unstarted();
    assert!(!action.handoff_status().engine.settled);
    drop((input, pressure));
    action.settle_engine();
    phase.close();
}
