//! Root's actual prepared LOW tail is lent before native locks, not under Group/Policy.
use super::*;
use crate::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use crate::voice_v2::sdp::{
    publication::{History, Prepared, Writer},
    SdpPublicationAdmission,
};

fn scoped(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    namespace: &mut ReceiptNamespace,
    outcome: Option<&mut Writer>,
    publish: impl FnOnce(SdpPublicationAdmission<'_>) -> Result<(), VoiceError>,
) -> Result<Arc<CryptoAction>, VoiceError> {
    let position = Some(namespace.reserve(0, 1)?);
    p.f.low.join.prepare_low_sdp_offer_scoped(
        &p.low,
        &room(&p.f.low),
        p.f.id,
        intent,
        b"v=0",
        position,
        namespace,
        &ca(&p.f.low.context, &p.f.low.lease),
        p._runtimes[0].clock(),
        &mut p.f.rng,
        |_, _| Ok(()),
        outcome,
        publish,
    )
}
#[test]
fn scoped_low_publication_keeps_prepared_owners_outside_refusal_and_unwind() {
    for mode in 0..3 {
        let (mut p, phase, intent, exchange) = fixture();
        let resources = p._runtimes[0].resources();
        let before = resources.counts();
        let packet = p.f.low.join.ledger.initial_offer_state_for_test(p.f.id);
        let mut namespace = ReceiptNamespace::default();
        let clock = p.origin_clock.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scoped(&mut p, &intent, &mut namespace, None, |step| {
                let _native = NativeFenceProbe::new(&clock);
                let prepared = resources.counts();
                {
                    let _uncommitted_loan = step;
                }
                assert_eq!(
                    resources.counts(),
                    prepared,
                    "dropping a loan destroyed prepared owners under native guards"
                );
                match mode {
                    0 => Err(VoiceError::Capacity),
                    1 => Ok(()),
                    _ => panic!("native final fence unwinds"),
                }
            })
        }));
        match mode {
            0 => assert_eq!(result.unwrap().err(), Some(VoiceError::Capacity)),
            1 => assert_eq!(result.unwrap().err(), Some(VoiceError::Unauthorized)),
            _ => assert!(result.is_err()),
        }
        assert_eq!(resources.counts(), before);
        assert_eq!(
            p.f.low.join.ledger.initial_offer_state_for_test(p.f.id),
            packet
        );
        assert!(!exchange.is_live());
        // No namespace/sequence/issue capacity was consumed by the refused tail.
        let action = offer(&mut p, &intent, &mut namespace, b"v=0", |_, _| Ok(())).unwrap();
        action.settle_engine();
        phase.close();
    }
}
#[test]
fn scoped_low_publication_binds_original_outcome_and_runs_no_clock_under_native_custody() {
    let (mut p, phase, intent, _) = fixture();
    let body = Some(crate::voice_v2::sdp::plaintext_commitment(b"v=0").unwrap());
    let slot = Prepared::new(
        intent.clone(),
        phase.native_history(NativeRowPurpose::Seal).unwrap(),
        body,
    )
    .unwrap()
    .install();
    let foreign = Prepared::new(
        intent.clone(),
        phase.native_history(NativeRowPurpose::Open).unwrap(),
        None,
    )
    .unwrap()
    .install();
    let mut writer = slot.try_writer().unwrap();
    writer.enter(&intent, body).unwrap();
    let clock = p.origin_clock.clone();
    let mut held = None;
    let action = scoped(
        &mut p,
        &intent,
        &mut ReceiptNamespace::default(),
        Some(&mut writer),
        |step| {
            assert!(step.belongs_to_outcome(&slot));
            assert!(!step.belongs_to_outcome(&foreign));
            assert_eq!(step.observed_time(), (1000, 1000));
            held = Some(NativeFenceProbe::new(&clock));
            step.commit()
        },
    )
    .unwrap();
    // The real injected Clock stayed fenced through all post-hook factory cleanup.
    drop(held);
    let History::Published(original) = slot.history() else {
        panic!("actual tail not recorded")
    };
    assert!(Arc::ptr_eq(&original, &action));
    assert!(!slot.permits_local_retry());
    action.settle_engine();
    phase.close();
}
#[test]
fn scoped_publish_then_callback_error_keeps_positive_original_outcome() {
    let (mut p, phase, intent, _) = fixture();
    let body = Some(crate::voice_v2::sdp::plaintext_commitment(b"v=0").unwrap());
    let slot = Prepared::new(
        intent.clone(),
        phase.native_history(NativeRowPurpose::Seal).unwrap(),
        body,
    )
    .unwrap()
    .install();
    let mut writer = slot.try_writer().unwrap();
    writer.enter(&intent, body).unwrap();
    let result = scoped(
        &mut p,
        &intent,
        &mut ReceiptNamespace::default(),
        Some(&mut writer),
        |step| {
            step.commit()?;
            Err(VoiceError::Capacity)
        },
    );
    assert_eq!(result.err(), Some(VoiceError::Capacity));
    writer.refused(VoiceError::Capacity);
    drop(writer);
    let History::Published(action) = slot.history() else {
        panic!("published became a false absence")
    };
    assert!(!slot.permits_local_retry());
    action.settle_engine();
    phase.close();
}
