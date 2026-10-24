//! Actual native Ready cache -> Source -> paid Lower lifecycle. Direct/helper outcomes here
//! exercise Source policy; signed transport ACK verification is covered by the Root fixture.
use super::*;
use crate::voice_v2::helper::BoundRoute;
use crate::voice_v2::negotiation::namespace::ReceiptNamespace;
use crate::voice_v2::source::{SourceHop, SourceOutcome};
use catcoms_rt::PeerId;
use futures::FutureExt;

fn install(p: &mut ProtectedFixture, helpers: &[DeviceId]) -> ReceiptNamespace {
    let signer = Signer::original(p);
    p.f.high
        .join
        .install_high_ready_receive(
            &p.high,
            &room(&p.f.high),
            p.f.id,
            helpers,
            &signer,
            &ca(&p.f.high.context, &p.f.high.lease),
            &p.f.clock,
            &mut Rng::new(),
            |_, _| Ok(()),
        )
        .unwrap();
    let mut namespace = ReceiptNamespace::default();
    p.f.high
        .join
        .attach_ready_position(p.f.id, namespace.reserve(0, 256).unwrap())
        .unwrap();
    namespace
}
fn queue(p: &ProtectedFixture, namespace: &ReceiptNamespace) -> Result<bool, VoiceError> {
    let (object, activity) = p.f.high.join.ready_source_activity(
        p.f.id,
        namespace,
        &ca(&p.f.high.context, &p.f.high.lease),
    )?;
    p.high
        .source()?
        .admit_borrowed(&p.high, object, activity, room(&p.f.high))
}

#[test]
fn native_ready_source_local_refusals_preserve_same_stamp_and_unspent_source_record() {
    let mut p = confirmed_high();
    let namespace = install(&mut p, &[]);
    let original = cache(&p).stamp_for_test().unwrap();
    let producer = cache(&p).producer_for_test();
    let occupied = producer
        .charge(
            &p._runtimes[1].resources(),
            p.f.high.context.scope.group,
            Stage::Crypto,
            1,
        )
        .unwrap();
    assert_eq!(queue(&p, &namespace), Err(VoiceError::Capacity));
    assert_eq!(p.high.source().unwrap().counts(), (0, 0, 0));
    drop(occupied);
    assert_eq!(
        p.high.with_limits(|_| queue(&p, &namespace)),
        Err(VoiceError::Capacity)
    );
    assert_eq!(
        p.high.with_held_policy_for_test(|| queue(&p, &namespace)),
        Err(VoiceError::Capacity)
    );
    assert_eq!(cache(&p).stamp_for_test().unwrap(), original);
    assert_eq!(queue(&p, &namespace), Ok(false));
    assert_eq!(queue(&p, &namespace), Ok(true));
    assert_eq!(cache(&p).stamp_for_test().unwrap(), original);
    assert_eq!(p.high.source().unwrap().counts(), (1, 0, 0));
}

#[test]
fn native_ready_source_each_final_native_clock_owner_close_refuses_before_origin_publication() {
    use std::sync::atomic::Ordering;
    let mut baseline = confirmed_high();
    let namespace = install(&mut baseline, &[]);
    let before = baseline.origin_clock.reads.load(Ordering::Acquire);
    queue(&baseline, &namespace).unwrap();
    let reads = baseline.origin_clock.reads.load(Ordering::Acquire) - before;
    assert!(reads > 0);
    for at in 1..=reads {
        let mut p = confirmed_high();
        let namespace = install(&mut p, &[]);
        let before = p.origin_clock.reads.load(Ordering::Acquire);
        *p.origin_clock.revoke.lock().unwrap() = Some((before + at, p.f.high.lease.clone()));
        assert!(
            queue(&p, &namespace).is_err(),
            "original clock boundary {at}"
        );
        assert_eq!(p.high.source().unwrap().counts(), (0, 0, 0));
    }
}

#[test]
fn native_ready_source_original_epoch_change_refuses_queued_retry_without_reviving_allowance() {
    let mut p = confirmed_high();
    let namespace = install(&mut p, &[]);
    queue(&p, &namespace).unwrap();
    let source = p.high.source().unwrap();
    let job = source
        .take_ready(&p.high, &routes(&[p.f.low.context.local]))
        .unwrap()
        .pop()
        .unwrap();
    p.high
        .sync_view(View {
            epoch: p.f.high.context.epoch + 1,
            members: [p.f.low.context.local, p.f.high.context.local]
                .into_iter()
                .collect(),
            policy: p.f.high.context.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    assert!(!job.authority().authorized());
    assert_eq!(queue(&p, &namespace), Err(VoiceError::Unauthorized));
    assert_eq!(source.counts(), (1, 1, 1));
    drop(job);
    assert_eq!(source.counts(), (0, 0, 0));
}

#[test]
fn native_ready_source_keeps_stamping_clock_when_preparation_has_a_different_monotonic_origin() {
    let mut p = confirmed_high();
    let preparation_clock = ManualClock::new(1000);
    preparation_clock.advance_ms(20000);
    preparation_clock.set_wall_ms(1000);
    let signer = Signer::original(&p);
    p.f.high
        .join
        .install_high_ready_receive(
            &p.high,
            &room(&p.f.high),
            p.f.id,
            &[],
            &signer,
            &ca(&p.f.high.context, &p.f.high.lease),
            &preparation_clock,
            &mut Rng::new(),
            |_, _| Ok(()),
        )
        .unwrap();
    let mut namespace = ReceiptNamespace::default();
    p.f.high
        .join
        .attach_ready_position(p.f.id, namespace.reserve(0, 256).unwrap())
        .unwrap();
    assert_eq!(
        cache(&p).stamp_for_test().unwrap().3,
        1000,
        "stamp uses the actual native actor clock"
    );
    assert_eq!(queue(&p, &namespace), Ok(false));
    preparation_clock.advance_ms(500000);
    assert_eq!(
        queue(&p, &namespace),
        Ok(true),
        "a foreign preparation origin cannot expire or renew Source"
    );
    let source = p.high.source().unwrap();
    let job = source
        .take_ready(&p.high, &routes(&[p.f.low.context.local]))
        .unwrap()
        .pop()
        .unwrap();
    assert!(job.authority().authorized());
    p.f.clock.advance_ms(15000);
    p.f.clock.set_wall_ms(1);
    assert!(!job.authority().authorized());
    assert_eq!(queue(&p, &namespace), Err(VoiceError::Unauthorized));
}
fn routes(ids: &[DeviceId]) -> Vec<BoundRoute> {
    ids.iter()
        .enumerate()
        .map(|(i, device)| BoundRoute {
            device: *device,
            peer: PeerId::from_u64(i as u64 + 1),
        })
        .collect()
}

#[tokio::test]
async fn native_ready_source_ignores_acks_preserves_exhaustion_then_exact_offer_wakes_actual_lower()
{
    let mut helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    helpers.sort();
    let rogue = DeviceKeypair::from_seed(&[90; 32]).device_id();
    let mut p = confirmed_high_in(fixture_with_members(&[helpers[0], helpers[1], rogue]));
    let namespace = install(&mut p, &helpers);
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let ordinary_crypto: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let pressure = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    let full = pool.counts();
    assert_eq!(
        queue(&p, &ReceiptNamespace::default()),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(queue(&p, &namespace), Ok(false));
    let source = p.high.source().unwrap();
    let routes = routes(&[p.f.low.context.local, helpers[0], helpers[1], rogue]);
    let mut late = None;
    let mut gate = None;
    for attempt in 1..=3 {
        let job = source.take_ready(&p.high, &routes).unwrap().pop().unwrap();
        let expected = match attempt {
            1 => SourceHop::Direct(p.f.low.context.local),
            2 => SourceHop::Helper(helpers[0]),
            _ => SourceHop::Helper(helpers[1]),
        };
        assert!(
            job.hop() == expected,
            "only the exact frozen helpers are eligible"
        );
        if attempt == 3 {
            late = Some(job.keepalive());
            gate = Some(job.authority());
        }
        let status = if attempt == 1 {
            crate::VoiceTransportStatus::EndpointAdmitted
        } else {
            crate::VoiceTransportStatus::HelperQueued
        };
        let outcome = source.complete(job.finish(Some(status))).unwrap();
        assert_eq!(
            outcome,
            if attempt == 3 {
                SourceOutcome::Unconfirmed
            } else {
                SourceOutcome::Pending
            }
        );
        assert_eq!(
            queue(&p, &namespace),
            Ok(true),
            "exact admission reply recovery never reissues Source"
        );
        p.f.clock.advance_ms(500);
    }
    assert_eq!(source.counts(), (1, 3, 1));
    assert!(source.take_ready(&p.high, &routes).unwrap().is_empty());
    let gate = gate.unwrap();
    let mut wait = Box::pin(gate.revoked());
    assert!(
        wait.as_mut().now_or_never().is_none(),
        "inactive Exchange is not cancellation"
    );
    let before = p.f.clock.monotonic_ms();
    let room = room(&p.f.high);
    let object = admission::offer(&p, 231, 4);
    admission::admit(&mut p, &object, &room, |_, _| Ok(())).unwrap();
    assert_eq!(
        pool.counts(),
        full,
        "maximum two-helper Source and actual first Offer use original payments"
    );
    assert!(!gate.authorized());
    assert!(
        wait.as_mut().now_or_never().is_some(),
        "actual Offer admission wakes Ready without a tick"
    );
    assert_eq!(p.f.clock.monotonic_ms(), before);
    assert_eq!(
        source.counts(),
        (1, 3, 1),
        "real Lower ownership survives causal completion"
    );
    assert_eq!(queue(&p, &namespace), Err(VoiceError::Unauthorized));
    drop(late);
    assert_eq!(source.counts(), (0, 0, 0));
    drop((ordinary_crypto, pressure));
}

#[test]
fn native_ready_source_full_ordinary_memory_crypto_and_exact_admission_replies_use_original_payments(
) {
    let mut p = confirmed_high();
    let namespace = install(&mut p, &[]);
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let crypto: Vec<_> = (0..3)
        .map(|_| pool.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let pressure = pool
        .charge(group, GROUP_BYTES - pool.counts().0, ResourceKind::Retained)
        .unwrap();
    let full = pool.counts();
    assert_eq!(queue(&p, &namespace), Ok(false));
    let source = p.high.source().unwrap();
    let routes = routes(&[p.f.low.context.local]);
    let first = source.take_ready(&p.high, &routes).unwrap().pop().unwrap();
    p.f.clock.advance_ms(500);
    let second = source.take_ready(&p.high, &routes).unwrap().pop().unwrap();
    p.f.clock.advance_ms(500);
    assert!(
        source.take_ready(&p.high, &routes).unwrap().is_empty(),
        "two actual Lower owners fence the third attempt"
    );
    for _ in 0..4 {
        assert_eq!(queue(&p, &namespace), Ok(true));
    }
    assert_eq!(pool.counts(), full);
    assert_eq!(source.counts(), (1, 2, 2));
    drop((first, second, crypto, pressure));
}

#[tokio::test]
async fn native_ready_source_strict_deadline_survives_wall_rollback_and_old_activity_cannot_rearm()
{
    let mut p = confirmed_high();
    let namespace = install(&mut p, &[]);
    queue(&p, &namespace).unwrap();
    let source = p.high.source().unwrap();
    let first = source
        .take_ready(&p.high, &routes(&[p.f.low.context.local]))
        .unwrap()
        .pop()
        .unwrap();
    let gate = first.authority();
    let mut wait = Box::pin(gate.revoked());
    assert!(wait.as_mut().now_or_never().is_none());
    p.f.clock.advance_ms(15000);
    p.f.clock.set_wall_ms(1);
    assert!(!gate.authorized());
    assert!(wait.as_mut().now_or_never().is_some());
    assert_eq!(queue(&p, &namespace), Err(VoiceError::Unauthorized));
    assert_eq!(source.counts(), (1, 1, 1));
    drop(first);
    assert_eq!(source.counts(), (0, 0, 0));
}

#[tokio::test]
async fn native_ready_source_end_cancels_without_waiting_for_collection_or_refunding_lower() {
    let mut p = confirmed_high();
    let namespace = install(&mut p, &[]);
    queue(&p, &namespace).unwrap();
    let source = p.high.source().unwrap();
    let first = source
        .take_ready(&p.high, &routes(&[p.f.low.context.local]))
        .unwrap()
        .pop()
        .unwrap();
    let gate = first.authority();
    let mut wait = Box::pin(gate.revoked());
    assert!(wait.as_mut().now_or_never().is_none());
    p.f.high.lease.revoke();
    assert!(!gate.authorized());
    assert!(wait.as_mut().now_or_never().is_some());
    assert_eq!(source.counts(), (1, 1, 1));
    drop(first);
    assert_eq!(source.counts(), (0, 0, 0));
}
